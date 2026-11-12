//! Strategic icons: zoomed out, each team's ships and stations show as solid
//! team-coloured icons at a fixed screen size (as in Sins of a Solar Empire and
//! Supreme Commander), replacing the model when it gets too small to make out.
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
//! - Each icon replaces its model below [`SYMBOL_ENTER_PX`] and returns to the
//!   model above [`MODEL_RETURN_PX`]. Tab cycles auto / always / off.
//! - Zoomed out past [`STRATEGIC_HEIGHT`] the view gains a grid on the
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

/// Projected-radius thresholds in logical pixels, with a hysteresis interval.
pub const SYMBOL_ENTER_PX: f32 = 7.0;
pub const MODEL_RETURN_PX: f32 = 10.0;

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
    /// Model radius after ScaleSOD; root/parent scale is applied when projecting.
    pub radius: f32,
    /// The model-bound centre in local Bevy coordinates.
    pub centre: Vec3,
    /// This frame's exclusive representation choice, including offscreen roots.
    pub symbol: bool,
    /// How much the icon shows this frame (0 hidden, 1 solid), and where.
    pub shown: f32,
    pub screen: Vec2,
    pub size_px: f32,
    /// The silhouette (white, tinted by `colour`), and its rectangle (0..1) when it
    /// is a sprite in an atlas; `None`: the generic `shape`.
    pub image: Option<Handle<Image>>,
    pub uv: Option<[f32; 4]>,
    owned_visibility: Option<Visibility>,
}

impl Strategic {
    pub fn new(shape: Shape, colour: Color, radius: f32) -> Self {
        Strategic { shape, colour, radius, centre: Vec3::ZERO, symbol: false, shown: 0.0, screen: Vec2::ZERO, size_px: icon_size(shape, radius), image: None, uv: None, owned_visibility: None }
    }

    pub fn with_centre(mut self, centre: Vec3) -> Self {
        self.centre = centre;
        self
    }

    /// With a silhouette image (`uv`: its rectangle in an atlas).
    pub fn with_image(mut self, image: Handle<Image>, uv: Option<[f32; 4]>) -> Self {
        self.image = Some(image);
        self.uv = uv;
        self.size_px = silhouette_size(self.radius);
        self
    }
}

/// Midpoint of imported model bounds after ScaleSOD, mirrored into Bevy's Z convention.
pub fn centre_for_bounds(lo: [f32; 3], hi: [f32; 3], scale: f32) -> Vec3 {
    let mid = (Vec3::from(lo) + Vec3::from(hi)) * (0.5 * scale);
    Vec3::new(mid.x, mid.y, -mid.z)
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

/// Choose one representation; retain the previous choice inside the gap.
pub fn symbol_for_radius(mode: Mode, previous: bool, px: f32) -> bool {
    match mode {
        Mode::Always => true,
        Mode::Off => false,
        Mode::Auto if previous => px < MODEL_RETURN_PX,
        Mode::Auto => px <= SYMBOL_ENTER_PX,
    }
}

fn gate_model(s: &mut Strategic, visibility: &mut Visibility, symbol: bool) -> bool {
    let externally_hidden = s.owned_visibility.is_none() && *visibility == Visibility::Hidden;
    if symbol && !externally_hidden {
        if s.owned_visibility.is_none() {
            s.owned_visibility = Some(*visibility);
        }
        *visibility = Visibility::Hidden;
    } else if let Some(original) = s.owned_visibility.take() {
        *visibility = original;
    }
    s.symbol = symbol;
    !externally_hidden
}

/// UI nodes use viewport-local coordinates scaled by `UiScale`.
fn icon_node_geometry(screen: Vec2, viewport_min: Vec2, size_px: f32, ui_scale: f32) -> (Vec2, f32) {
    let scale = if ui_scale > 0.0 { ui_scale } else { 1.0 };
    ((screen - viewport_min - Vec2::splat(size_px * 0.5)) / scale, size_px / scale)
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
            .add_systems(PostUpdate, draw.run_if(resource_exists::<bevy::gizmos::config::GizmoConfigStore>).after(update))
            // The handoff precedes both model visibility propagation and UI layout.
            .add_systems(
                PostUpdate,
                update.after(TransformSystems::Propagate)
                    .after(bevy::camera::CameraUpdateSystems)
                    .before(bevy::camera::visibility::VisibilitySystems::VisibilityPropagate)
                    .before(bevy::ui::UiSystems::Prepare),
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
    camera: Query<(Entity, &Camera, &GlobalTransform, &Projection), crate::map::MainCamera>,
    mut objects: Query<(&GlobalTransform, &mut Strategic, &mut Visibility, Option<&SimEntity>), With<Strategic>>,
    selection: Res<Selection>,
    mut icons: Query<(Entity, &Icon, &mut Node, &mut ImageNode, &mut UiTransform, Option<&UiTargetCamera>)>,
    mut outlines: Query<&mut Visibility, (Without<Icon>, Without<Strategic>)>,
    mut texts: Query<&mut Text>,
    mut sv: ResMut<StrategicView>,
    images: Res<Assets<Image>>,
    ui_scale: Option<Res<UiScale>>,
) {
    let Ok((camera_e, cam, cam_tf, proj)) = camera.single() else {
        clear_unprojected(&mut objects, &mut icons, &mut sv);
        return;
    };
    let Some(view) = cam.logical_viewport_size() else {
        clear_unprojected(&mut objects, &mut icons, &mut sv);
        return;
    };
    let Some(viewport) = cam.logical_viewport_rect() else {
        clear_unprojected(&mut objects, &mut icons, &mut sv);
        return;
    };
    let fov = match proj {
        Projection::Perspective(p) => p.fov,
        _ => 0.8,
    };
    // Pixels per world unit at distance 1.
    let focal = view.y * 0.5 / (fov * 0.5).tan();
    let eye = cam_tf.translation();
    let fwd = cam_tf.forward().as_vec3();
    let scale = ui_scale.map_or(1.0, |s| if s.0 > 0.0 { s.0 } else { 1.0 });
    sv.active = match *mode {
        Mode::Always => true,
        Mode::Off => false,
        Mode::Auto => eye.y.abs() > STRATEGIC_HEIGHT,
    };
    sv.grid_lines = if sv.active { (GRID_HALF_CELLS as usize * 2 + 1) * 2 } else { 0 };
    sv.icons.clear();
    sv.bars.clear();
    sv.folded.clear();
    let mut list = Vec::new();
    for (gt, mut s, mut visibility, sim) in &mut objects {
        let pos = gt.transform_point(s.centre);
        let depth = (pos - eye).dot(fwd);
        let screen = if depth > 1.0 { cam.world_to_viewport(cam_tf, pos).ok().filter(|p| viewport.contains(*p)) } else { None };
        let root_scale = gt.to_scale_rotation_translation().0.abs().max_element();
        let projected_radius = if depth > 1.0 { s.radius * root_scale * focal / depth } else { f32::INFINITY };
        let symbol = symbol_for_radius(*mode, s.symbol, projected_radius);
        let eligible = gate_model(&mut s, &mut visibility, symbol);
        s.shown = if symbol && eligible && screen.is_some() { 1.0 } else { 0.0 };
        if let Some(p) = screen {
            s.screen = p;
        }
        if s.shown > 0.0
            && let Some(sim) = sim
        {
            list.push((sim.0, pos));
        }
    }
    sv.icons = stack(&list, eye, fwd, focal, STACK_RADIUS_PX);
    let folded: Vec<EntityId> = sv.icons.iter().flat_map(|i| i.members.iter().skip(1).copied()).collect();
    sv.folded.extend(folded);
    if sv.active {
        sv.bars = list.iter().map(|&(unit, p)| HeightBar { unit, top: p, bottom: Vec3::new(p.x, 0.0, p.z) }).collect();
        sv.bars.sort_by_key(|b| b.unit);
    }
    for (icon_e, icon, mut node, mut img, mut ui_tf, target_camera) in &mut icons {
        if target_camera.is_none_or(|target| target.0 != camera_e) {
            commands.entity(icon_e).insert(UiTargetCamera(camera_e));
        }
        let Ok((gt, mut s, _, sim)) = objects.get_mut(icon.target) else {
            commands.entity(icon_e).despawn();
            continue;
        };
        let selected = sim.is_some_and(|e| selection.0.contains(&e.0));
        let Some(p) = (s.shown > 0.0).then_some(s.screen) else {
            node.display = Display::None;
            continue;
        };
        let id = sim.map(|e| e.0);
        // Folded into a fleet icon: only the leader draws.
        let folded = id.is_some_and(|i| sv.folded.contains(&i));
        if folded {
            s.shown = 0.0;
        }
        let count = id.and_then(|i| sv.icons.iter().find(|s| s.leader == i)).map_or(1, StackIcon::count);
        if let Ok(mut v) = outlines.get_mut(icon.badge) {
            *v = if count > 1 && !folded { Visibility::Inherited } else { Visibility::Hidden };
        }
        if count > 1
            && let Ok(mut t) = texts.get_mut(icon.badge)
            && t.0 != count.to_string()
        {
            t.0 = count.to_string();
        }
        node.display = if folded { Display::None } else { Display::Flex };
        let (ui_pos, ui_size) = icon_node_geometry(p, viewport.min, s.size_px, scale);
        node.width = Val::Px(ui_size);
        node.height = Val::Px(ui_size);
        node.left = Val::Px(ui_pos.x);
        node.top = Val::Px(ui_pos.y);
        img.color = s.colour;
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
            let pos = gt.transform_point(s.centre);
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

#[allow(clippy::type_complexity)]
fn clear_unprojected(
    objects: &mut Query<(&GlobalTransform, &mut Strategic, &mut Visibility, Option<&SimEntity>), With<Strategic>>,
    icons: &mut Query<(Entity, &Icon, &mut Node, &mut ImageNode, &mut UiTransform, Option<&UiTargetCamera>)>,
    view: &mut StrategicView,
) {
    view.active = false;
    view.icons.clear();
    view.bars.clear();
    view.folded.clear();
    view.grid_lines = 0;
    for (_, mut s, mut visibility, _) in objects.iter_mut() {
        s.shown = 0.0;
        s.symbol = false;
        if let Some(original) = s.owned_visibility.take() {
            *visibility = original;
        }
    }
    for (_, _, mut node, _, _, _) in icons.iter_mut() {
        node.display = Display::None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_handoff_has_a_hysteresis_gap_and_mode_overrides() {
        assert!(!symbol_for_radius(Mode::Auto, false, 7.01));
        assert!(symbol_for_radius(Mode::Auto, false, 7.0));
        assert!(symbol_for_radius(Mode::Auto, true, 9.99));
        assert!(!symbol_for_radius(Mode::Auto, true, 10.0));
        assert!(symbol_for_radius(Mode::Always, false, 100.0));
        assert!(!symbol_for_radius(Mode::Off, true, 0.0));
    }

    #[test]
    fn model_bounds_centre_uses_scale_and_z_mirror() {
        assert_eq!(centre_for_bounds([2.0, -4.0, 6.0], [6.0, 2.0, 10.0], 2.0), Vec3::new(8.0, -2.0, -16.0));
        assert_eq!(Strategic::new(Shape::Square, Color::WHITE, 2.0).with_centre(Vec3::X).centre, Vec3::X);
    }

    #[test]
    fn root_visibility_is_restored_only_when_owned() {
        // No SimEntity is needed: this is the same gate used for static roots.
        let mut static_root = Strategic::new(Shape::Square, Color::WHITE, 20.0);
        let mut visible = Visibility::Visible;
        assert!(gate_model(&mut static_root, &mut visible, true));
        assert_eq!(visible, Visibility::Hidden);
        assert!(static_root.symbol);
        assert!(gate_model(&mut static_root, &mut visible, false));
        assert_eq!(visible, Visibility::Visible);
        let mut hidden = Visibility::Hidden;
        assert!(!gate_model(&mut static_root, &mut hidden, true));
        assert_eq!(hidden, Visibility::Hidden);
        assert_eq!(static_root.owned_visibility, None);
    }

    #[test]
    fn ui_geometry_is_viewport_local_and_unscaled() {
        let (pos, size) = icon_node_geometry(Vec2::new(500.0, 350.0), Vec2::new(100.0, 50.0), 20.0, 2.0);
        assert_eq!(pos, Vec2::new(195.0, 145.0));
        assert_eq!(size, 10.0);
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
    use bevy::camera::{CameraProjection, ComputedCameraValues, RenderTargetInfo};
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
            .add_plugins(bevy::transform::TransformPlugin)
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
        let mut projection = PerspectiveProjection::default();
        projection.update(1280.0, 720.0);
        let camera = Camera {
            computed: ComputedCameraValues {
                clip_from_view: projection.get_clip_from_view(),
                target_info: Some(RenderTargetInfo { physical_size: UVec2::new(1280, 720), scale_factor: 1.0 }),
                ..default()
            },
            ..default()
        };
        app.world_mut().spawn((Camera3d::default(), camera, Projection::Perspective(projection), Transform::from_xyz(0.0, cam_y, 0.0).looking_to(Vec3::NEG_Y, Vec3::NEG_Z)));
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

    fn camera_height(app: &mut App, y: f32) {
        let mut q = app.world_mut().query_filtered::<&mut Transform, With<Camera3d>>();
        q.single_mut(app.world_mut()).unwrap().translation.y = y;
        app.update();
    }

    fn static_state(app: &mut App, root: Entity) -> (Visibility, bool, f32, Display) {
        let s = app.world().get::<Strategic>(root).unwrap();
        let visibility = *app.world().get::<Visibility>(root).unwrap();
        let symbol = s.symbol;
        let shown = s.shown;
        let mut icons = app.world_mut().query::<(&Icon, &Node)>();
        let display = icons.iter(app.world()).find(|(i, _)| i.target == root).unwrap().1.display;
        (visibility, symbol, shown, display)
    }

    #[test]
    fn static_roots_handoff_exclusively_at_projected_thresholds() {
        let mut app = rig(&world(&[]), None, 1600.0);
        let small = app.world_mut().spawn((Strategic::new(Shape::Arrow, Color::WHITE, 20.0), Transform::default(), Visibility::Inherited)).id();
        let large = app.world_mut().spawn((Strategic::new(Shape::Square, Color::WHITE, 300.0), Transform::from_xyz(400.0, 0.0, 0.0), Visibility::Inherited)).id();
        app.update();
        app.update();
        assert_eq!(static_state(&mut app, small), (Visibility::Inherited, false, 0.0, Display::None));
        camera_height(&mut app, 2500.0); // ~6.8 px for the small root
        assert_eq!(static_state(&mut app, small), (Visibility::Hidden, true, 1.0, Display::Flex));
        assert_eq!(static_state(&mut app, large), (Visibility::Inherited, false, 0.0, Display::None));
        camera_height(&mut app, 2100.0); // ~8.1 px: retain symbol
        assert_eq!(static_state(&mut app, small), (Visibility::Hidden, true, 1.0, Display::Flex));
        camera_height(&mut app, 1800.0); // ~9.5 px: still symbol
        assert_eq!(static_state(&mut app, small), (Visibility::Hidden, true, 1.0, Display::Flex));
        camera_height(&mut app, 1600.0); // >10 px: restore model
        assert_eq!(static_state(&mut app, small), (Visibility::Inherited, false, 0.0, Display::None));
    }

    #[test]
    fn hidden_static_root_stays_hidden_and_camera_loss_restores_owned_visibility() {
        let mut app = rig(&world(&[]), None, 300.0);
        let hidden = app.world_mut().spawn((Strategic::new(Shape::Square, Color::WHITE, 20.0), Transform::default(), Visibility::Hidden)).id();
        let owned = app.world_mut().spawn((Strategic::new(Shape::Arrow, Color::WHITE, 20.0), Transform::from_xyz(200.0, 0.0, 0.0), Visibility::Visible)).id();
        app.update();
        *app.world_mut().resource_mut::<Mode>() = Mode::Always;
        app.update();
        assert_eq!(static_state(&mut app, hidden), (Visibility::Hidden, true, 0.0, Display::None));
        assert_eq!(static_state(&mut app, owned), (Visibility::Hidden, true, 1.0, Display::Flex));
        *app.world_mut().resource_mut::<Mode>() = Mode::Off;
        app.update();
        assert_eq!(static_state(&mut app, hidden), (Visibility::Hidden, false, 0.0, Display::None));
        assert_eq!(static_state(&mut app, owned), (Visibility::Visible, false, 0.0, Display::None));
        *app.world_mut().resource_mut::<Mode>() = Mode::Always;
        app.update();
        let mut q = app.world_mut().query_filtered::<Entity, With<Camera3d>>();
        let camera = q.single(app.world()).unwrap();
        app.world_mut().despawn(camera);
        app.update();
        assert_eq!(static_state(&mut app, hidden), (Visibility::Hidden, false, 0.0, Display::None));
        assert_eq!(static_state(&mut app, owned), (Visibility::Visible, false, 0.0, Display::None));
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
        assert_eq!(view(&all).icons.len(), 2);
        let mut q = all.world_mut().query::<(&SimEntity, &Strategic)>();
        assert_eq!(q.iter(all.world()).find(|(e, _)| e.0 == EntityId(2)).unwrap().1.shown, 0.0);
        let mut mine = rig(&world(&us), Some(PlayerId(1)), 6000.0);
        mine.update();
        mine.update();
        let drawn: Vec<EntityId> = view(&mine).icons.iter().flat_map(|icon| icon.members.iter().copied()).collect();
        let seen: Vec<EntityId> = mine.world().resource::<SimLink>().curr().visible_to(PlayerId(1)).iter().map(|u| u.id).collect();
        assert!(drawn.iter().all(|id| seen.contains(id)));
        assert_eq!(drawn, vec![EntityId(1), EntityId(3)]);
    }
}

#[cfg(test)]
#[path = "strategic_gpu.rs"]
mod gpu_tests;
