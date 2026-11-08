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
//! - Selected units' icons get a green outline; clicks and drag boxes select by
//!   icon when it shows (see `units::select`).

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
}

pub struct StrategicPlugin;

impl Plugin for StrategicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mode>()
            .add_systems(Startup, make_masks)
            .add_systems(Update, (toggle, attach).chain())
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
        let image = s.image.clone().unwrap_or_else(|| masks.fill[i].clone());
        commands
            .spawn((
                Icon { target: e, outline },
                ImageNode { image, color: s.colour, ..default() },
                node,
                UiTransform::IDENTITY,
                // Under the HUD text, over the scene.
                GlobalZIndex(-1),
            ))
            .add_child(outline);
    }
}

#[allow(clippy::type_complexity)]
fn update(
    mut commands: Commands,
    mode: Res<Mode>,
    camera: Query<(&Camera, &GlobalTransform, &Projection), crate::map::MainCamera>,
    mut objects: Query<(&GlobalTransform, &mut Strategic, Has<crate::units::Selected>)>,
    mut icons: Query<(Entity, &Icon, &mut Node, &mut ImageNode, &mut UiTransform)>,
    mut outlines: Query<&mut Visibility, Without<Icon>>,
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
        let Ok((gt, mut s, selected)) = objects.get_mut(icon.target) else {
            commands.entity(icon_e).despawn();
            continue;
        };
        let pos = gt.translation();
        let depth = (pos - eye).dot(fwd);
        let screen = if depth > 1.0 { cam.world_to_viewport(cam_tf, pos).ok() } else { None };
        let shown = match (*mode, screen) {
            (_, None) | (Mode::Off, _) => 0.0,
            (Mode::Always, _) => 1.0,
            (Mode::Auto, _) => fade(s.radius * focal / depth),
        };
        s.shown = shown;
        let Some(p) = screen.filter(|_| shown > 0.0) else {
            node.display = Display::None;
            continue;
        };
        s.screen = p;
        node.display = Display::Flex;
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
}
