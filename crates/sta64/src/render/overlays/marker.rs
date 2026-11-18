//! RND-37: the Armada I selection marker, a flat reticle under a selected or hovered unit.
//!
//! Rules (evidence grades in brackets; "static" is read from the Armada I executable):
//! - Art: the sprite `selectionCircle` of `sprites/gui_global.spr`, through the mounted VFS, so a mod
//!   that redefines the sprite or its texture changes the marker [read from data]. The engine looks
//!   up `selectionCircle` and `selectionBillboard` by name when it builds its GUI sprite set
//!   [static]; the sprite's material is additive (`add_nomipf`) [read from data].
//! - When: for every unit that is selected, and for the unit under the cursor. One marker per unit, so a
//!   group gets one each [static]. The engine's per-object test also has three further conditions whose
//!   meaning is not established (alive, visible, not hidden are the plainest reading); the routine
//!   compares neither owner nor relation, so no relation is excluded [static, not measured in play].
//! - Size and place: a square of side twice the unit's radius, lying in the horizontal plane through
//!   the unit's position lowered by one radius, axis-aligned in the world (it does not turn with the
//!   ship), no animation [static]. STA64 takes the radius from [`SelectRadius`]; whether that equals the
//!   engine's radius field is not measured.
//! - Colour: the engine hands the sprite a four-float colour taken from the object; which field it is
//!   was not established, and the frame shows the texture's own grey, so no tint is applied
//!   [not measured: other relations or states may tint it].
//! - Depth: drawn additive, depth-tested and not writing depth [not measured; the frame shows it under
//!   the ship].
//! - Not drawn: the engine's second branch (camera-facing `selectionBillboard`, taken on one value of its render
//!   mode) [static; when that mode applies is not measured].
//!
//! The `SELECTION_CIRCLE_*` keys of `RTS_CFG.h` are parsed by the engine but their values are never read
//! [static], so they are not honoured here. Active only under the Armada I stock presentation.

use bevy::image::CompressedImageFormats;
use bevy::math::Affine2;
use bevy::prelude::*;

use crate::ui::gui_sprites::{self, GuiImageSampling, GuiSprites};
use crate::ui::profile::UiProfile;
use crate::viewer::Mounted;

/// Name of the marker sprite in the global GUI sprite table.
pub const SPRITE: &str = "selectionCircle";

/// The marker's art, when the Armada I stock presentation is on and the install defines it.
#[derive(Resource, Default)]
pub struct MarkerArt {
    pub art: Option<Art>,
    tried_for_mount: bool,
}

#[derive(Clone)]
pub struct Art {
    pub image: Handle<Image>,
    /// UV transform that maps the unit quad onto the sprite's rectangle.
    pub uv: Affine2,
}

/// A marker quad under unit root `ChildOf`: `radius` is what it was sized by.
#[derive(Component, Debug, Clone, Copy)]
pub struct Marker {
    pub radius: f32,
}

pub(super) fn wanted(profile: Option<&UiProfile>) -> bool {
    profile.is_some_and(|p| *p == UiProfile::Armada1)
}

/// UV transform for a pixel rectangle `[min, max]` of an image of `size`.
pub(super) fn uv_of(rect: Rect, size: UVec2) -> Affine2 {
    let (w, h) = (size.x as f32, size.y as f32);
    Affine2 { matrix2: Mat2::from_diagonal(Vec2::new(rect.width() / w, rect.height() / h)), translation: Vec2::new(rect.min.x / w, rect.min.y / h) }
}

pub(super) fn load_art(
    mut art: ResMut<MarkerArt>,
    profile: Option<Res<UiProfile>>,
    mounted: Option<Res<Mounted>>,
    images: Option<ResMut<Assets<Image>>>,
) {
    let on = wanted(profile.as_deref());
    let (Some(mounted), Some(mut images)) = (mounted, images) else {
        if art.art.is_some() {
            art.art = None;
        }
        return;
    };
    if !on {
        if art.art.is_some() || art.tried_for_mount {
            art.art = None;
            art.tried_for_mount = false;
        }
        return;
    }
    if mounted.is_changed() {
        art.tried_for_mount = false;
    }
    if art.tried_for_mount {
        return;
    }
    art.tried_for_mount = true;
    let table = gui_sprites::read_table(&mounted.0, "sprites/gui_global.spr");
    art.art = table.sprite(SPRITE).and_then(|sprite| {
        let mut cache = GuiSprites::default();
        let (image, size) = gui_sprites::page(sprite, GuiImageSampling::Linear, &mounted.0, &mut cache, &mut images, CompressedImageFormats::empty())?;
        let rect = gui_sprites::crop(sprite, size)?;
        Some(Art { image, uv: uv_of(rect, size) })
    });
}

pub(super) fn material(art: &Art) -> StandardMaterial {
    StandardMaterial {
        base_color_texture: Some(art.image.clone()),
        uv_transform: art.uv,
        unlit: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Add,
        ..default()
    }
}

/// Keep each marker axis-aligned in the world, one radius below its unit.
pub(super) fn orient(parents: Query<&Transform, Without<Marker>>, mut markers: Query<(&Marker, &ChildOf, &mut Transform)>) {
    for (m, p, mut tf) in &mut markers {
        let Ok(parent) = parents.get(p.parent()) else { continue };
        let inverse = parent.rotation.inverse();
        let next = Transform {
            translation: inverse * Vec3::new(0.0, -m.radius, 0.0),
            rotation: inverse * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            scale: Vec3::new(2.0 * m.radius, 2.0 * m.radius, 1.0),
        };
        if *tf != next {
            *tf = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_is_for_the_armada_one_stock_presentation_only() {
        assert!(wanted(Some(&UiProfile::Armada1)));
        assert!(!wanted(Some(&UiProfile::Armada2)));
        assert!(!wanted(Some(&UiProfile::FleetOps)));
        assert!(!wanted(Some(&UiProfile::Lcars)));
        assert!(!wanted(None));
    }

    #[test]
    fn uv_maps_the_sprite_rectangle() {
        let full = uv_of(Rect::new(0.0, 0.0, 128.0, 128.0), UVec2::splat(128));
        assert_eq!(full.transform_point2(Vec2::new(0.25, 0.75)), Vec2::new(0.25, 0.75));
        let quarter = uv_of(Rect::new(64.0, 0.0, 128.0, 64.0), UVec2::splat(128));
        assert_eq!(quarter.transform_point2(Vec2::new(0.0, 0.0)), Vec2::new(0.5, 0.0));
        assert_eq!(quarter.transform_point2(Vec2::new(1.0, 1.0)), Vec2::new(1.0, 0.5));
    }

    #[test]
    fn marker_stays_flat_and_world_aligned_under_a_turning_unit() {
        let mut app = App::new();
        let root = app.world_mut().spawn(Transform::from_rotation(Quat::from_rotation_y(0.7) * Quat::from_rotation_z(0.3))).id();
        let m = app.world_mut().spawn((Marker { radius: 10.0 }, ChildOf(root), Transform::default())).id();
        app.add_systems(Update, orient);
        app.update();
        let parent = *app.world().get::<Transform>(root).unwrap();
        let local = *app.world().get::<Transform>(m).unwrap();
        let world = parent * local;
        assert!((world.translation - Vec3::new(0.0, -10.0, 0.0)).length() < 1e-4, "one radius below the unit");
        assert!((world.rotation * Vec3::Z - Vec3::Y).length() < 1e-4, "faces up in the world");
        assert_eq!(local.scale, Vec3::new(20.0, 20.0, 1.0));
    }
}
