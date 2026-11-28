//! The Armada II hover readout's system icons (#1163), Stock look only, through [`super::armada2_look`]: a row of five
//! icons under a hovered craft.
//!
//! Rules (grades: "measured" is from frames of the original on the modded bench; "static" read from the executable):
//! - **Shown** for a craft while it is hovered and the readout's bars are up (the same gate as the bars; static). A
//!   craft that is only selected, with the pointer elsewhere, shows none (measured, one window size, one frame). The
//!   row is a state row, not only a damage row: a craft without hull damage whose systems are switched off shows it too
//!   (measured, one frame).
//! - **Place:** the row is centred on the craft's projected box, under it ([`BELOW_BOX`] below its bottom edge, static),
//!   and its spacing and offset scale with the window like the rest of the HUD (measured, two window sizes: both are in
//!   units of an 800x600 window, [`DESIGN_HEIGHT`]).
//! - **Art:** the sprite [`SPRITE`] of the install's global GUI sprite table, read at run time; none drawn without it.
//! - **Looks:** each icon takes one of several looks (measured: two alert tints, a dim one, once a partial one). INTERIM:
//!   which look means which level, which icon is which system and the icons' order are not established (the bench could
//!   not read the health of a system); the order is that of the static read (engines, life support, sensors, shields,
//!   weapons), and the looks are mapped to offline, damaged and fine by [`look_of`].
//! - Interim: none while a cineractive runs (not established for the original).

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;
use bevy::ui::UiScale;
use sta_sim::EntityId;
use sta_sim::appearance::SystemAppearance;

use super::readout::{self, RowArt};
use super::{DEFAULT_RADIUS, ModelBounds, armada2_look, geometry};
use crate::input::hover::HoverReadout;
use crate::render::units::SelectRadius;
use crate::sim_link::{ContentRes, SimEntities, SimLink};
use crate::ui::gui_sprites::{self, GuiImageSampling, GuiSprites};
use crate::ui::profile::UiProfile;
use crate::viewer::Mounted;

/// The sprite key of one system icon in the global GUI sprite table.
pub const SPRITE: &str = "select_system";
/// The window height the icon layout is laid out in (the HUD's design height; measured: the layout scales with it).
pub const DESIGN_HEIGHT: f32 = 600.0;
/// The distance between neighbouring icon centres, in design units (measured, two window sizes).
pub const PITCH: f32 = 14.0;
/// The gap between the box's bottom edge and the icons, in design units (read from the executable, static).
pub const BELOW_BOX: f32 = 32.0;
/// The number of systems (static).
pub const COUNT: usize = 5;

/// How one icon looks. INTERIM mapping, see the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// Switched off or destroyed.
    Offline,
    /// Damaged: below its full health.
    Damaged,
    /// Working and undamaged.
    Fine,
}

/// The look of a system (INTERIM mapping).
pub fn look_of(s: &SystemAppearance) -> Look {
    if !s.enabled || s.health[0].to_f32() <= 0.0 {
        Look::Offline
    } else if s.health[0] < s.health[1] {
        Look::Damaged
    } else {
        Look::Fine
    }
}

/// The tint of a look: the existing readout palette of the stock look's strip (INTERIM; the original's tints are
/// not in the repository).
pub fn tint(look: Look) -> Color {
    match look {
        Look::Offline => Color::srgb(0.2, 0.2, 0.2),
        Look::Damaged => Color::srgb(1.0, 1.0, 0.0),
        Look::Fine => Color::WHITE,
    }
}

/// Whether the hovered unit `id` shows its row: it is the hovered one and the bars are up.
pub fn shows(readout: &HoverReadout, id: EntityId) -> bool {
    readout.bars_up() && readout.unit == Some(id)
}

/// The five icon centres for a unit box, in camera pixels: centred on the box, under it, scaled with the window.
pub fn row_centres(b: Rect, window_height: f32) -> [Vec2; COUNT] {
    let s = window_height / DESIGN_HEIGHT;
    let y = b.max.y + BELOW_BOX * s;
    std::array::from_fn(|i| Vec2::new(b.center().x + (i as f32 - (COUNT as f32 - 1.0) / 2.0) * PITCH * s, y))
}

/// The art of the icons, when the Armada II look is on and the install defines the sprite.
#[derive(Resource, Default)]
pub struct SystemArt {
    pub art: Option<RowArt>,
    tried_for_mount: bool,
}

pub(super) fn load_art(mut art: ResMut<SystemArt>, profile: Option<Res<UiProfile>>, mounted: Option<Res<Mounted>>, images: Option<ResMut<Assets<Image>>>) {
    let (Some(mounted), Some(mut images)) = (mounted, images) else {
        art.art = None;
        return;
    };
    if !armada2_look(profile.as_deref()) {
        art.art = None;
        art.tried_for_mount = false;
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
    let mut cache = GuiSprites::default();
    art.art = table.sprite(SPRITE).and_then(|sprite| {
        let (image, size) = gui_sprites::page(sprite, GuiImageSampling::Linear, &mounted.0, &mut cache, &mut images, bevy::image::CompressedImageFormats::empty())?;
        let rect = gui_sprites::crop(sprite, size)?;
        Some(RowArt { image, rect, size })
    });
}

/// One icon's node: `index` 0 to 4 of the unit's row.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SystemIcon {
    pub unit: EntityId,
    pub index: u8,
}

/// One icon to show: its centre in camera pixels and its look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub centre: Vec2,
    pub look: Look,
}

/// The nodes of the icons.
pub type IconQuery<'w, 's> = Query<'w, 's, (Entity, &'static SystemIcon, &'static mut Node, &'static mut ImageNode)>;

/// Bring the UI nodes in line with `want`, keeping the ones that stay. `size` is an icon's size in camera pixels,
/// `ui_scale` converts camera pixels to UI pixels.
pub fn reconcile(commands: &mut Commands, existing: &mut IconQuery, want: &BTreeMap<SystemIcon, Placed>, art: &RowArt, size: Vec2, ui_scale: f32) {
    let at = |p: &Placed| ((p.centre.x - size.x / 2.0) / ui_scale, (p.centre.y - size.y / 2.0) / ui_scale);
    let mut have = BTreeSet::new();
    for (e, icon, mut node, mut image) in existing.iter_mut() {
        let Some(p) = want.get(icon).filter(|_| have.insert(*icon)) else {
            commands.entity(e).try_despawn();
            continue;
        };
        let (left, top) = at(p);
        let (left, top) = (Val::Px(left), Val::Px(top));
        if node.left != left {
            node.left = left;
        }
        if node.top != top {
            node.top = top;
        }
        let colour = tint(p.look);
        if image.color != colour {
            image.color = colour;
        }
    }
    for (icon, p) in want {
        if have.contains(icon) {
            continue;
        }
        let (left, top) = at(p);
        let mut image = ImageNode::new(art.image.clone()).with_color(tint(p.look));
        image.rect = Some(art.rect);
        commands.spawn((
            Name::new("system icon"),
            *icon,
            Node { position_type: PositionType::Absolute, left: Val::Px(left), top: Val::Px(top), width: Val::Px(size.x / ui_scale), height: Val::Px(size.y / ui_scale), ..default() },
            image,
            bevy::picking::Pickable::IGNORE,
        ));
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn sync_icons(
    mut commands: Commands,
    profile: Option<Res<UiProfile>>,
    readout: Res<HoverReadout>,
    art: Res<SystemArt>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    ui_scale: Option<Res<UiScale>>,
    content: Option<Res<ContentRes>>,
    link: Res<SimLink>,
    ents: Res<SimEntities>,
    cam: Query<(Entity, &Camera, &Projection), crate::map::MainCamera>,
    parents: Query<(Option<&ModelBounds>, Option<&SelectRadius>)>,
    helper: TransformHelper,
    mut existing: IconQuery,
) {
    let mut want: BTreeMap<SystemIcon, Placed> = BTreeMap::new();
    let mut scale = 1.0;
    let mut size = Vec2::ZERO;
    if armada2_look(profile.as_deref())
        && !crate::ui::presentation::cutscene_running(cine)
        && let Some(art) = art.art.as_ref()
        && let Some(id) = readout.unit.filter(|id| shows(&readout, *id))
        && let Some(unit) = link.curr().unit(id)
        && content.as_deref().and_then(|c| c.0.unit(unit.def)).is_some_and(|d| sta_sim::world::EntityKind::of_def(d) == sta_sim::world::EntityKind::Craft)
        && let Ok((cam_entity, camera, projection)) = cam.single()
        && let (Some(window), Ok(transform)) = (camera.logical_viewport_size(), helper.compute_global_transform(cam_entity))
        && let Some(&root) = ents.0.get(&id)
        && let Ok(world) = helper.compute_global_transform(root)
        && let Ok((bounds, radius)) = parents.get(root)
    {
        let origin = camera.logical_viewport_rect().map_or(Vec2::ZERO, |r| r.min);
        let clip = projection.get_clip_from_view() * transform.to_matrix().inverse();
        scale = ui_scale.map_or(1.0, |s| s.0).max(1e-3);
        let bounds = bounds.copied().unwrap_or_else(|| ModelBounds::fallback(radius.map_or(DEFAULT_RADIUS, |r| r.0)));
        if let Some((b, _)) = geometry::project(bounds, world.to_matrix(), clip, window) {
            let b = Rect { min: b.min + origin, max: b.max + origin };
            let sys = link.curr().appearance.systems.binary_search_by_key(&id, |(u, _)| *u).ok().map(|i| link.curr().appearance.systems[i].1);
            let s = window.y / DESIGN_HEIGHT;
            size = art.rect.size() * s;
            for (i, centre) in row_centres(b, window.y).into_iter().enumerate() {
                // A system the class does not have draws no icon.
                if let Some(look) = sys.and_then(|a| a[i]).map(|a| look_of(&a)) {
                    want.insert(SystemIcon { unit: id, index: i as u8 }, Placed { centre, look });
                }
            }
        }
    }
    match art.art.as_ref() {
        Some(a) => reconcile(&mut commands, &mut existing, &want, a, size, scale),
        None => {
            for (e, ..) in existing.iter() {
                commands.entity(e).try_despawn();
            }
        }
    }
    let _ = readout::MAX_ABOVE;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use sta_sim::{Fixed, PlayerId};

    fn sys(enabled: bool, now: i32, max: i32) -> SystemAppearance {
        SystemAppearance { enabled, health: [Fixed::from_int(now), Fixed::from_int(max)] }
    }

    fn hov(unit: u32, fade: f32) -> HoverReadout {
        HoverReadout { unit: Some(EntityId(unit)), team: Some(PlayerId(1)), fade }
    }

    #[test]
    fn the_row_shows_for_the_hovered_craft_once_the_bars_are_up_and_never_for_a_selection_alone() {
        assert!(shows(&hov(1, 0.75), EntityId(1)));
        assert!(!shows(&hov(1, 0.75), EntityId(2)), "another unit of the same team: no row");
        assert!(!shows(&hov(1, 0.5), EntityId(1)), "the bars are not up yet");
        assert!(!shows(&HoverReadout::default(), EntityId(1)), "nothing hovered: a selected craft shows none");
    }

    #[test]
    fn the_row_is_centred_under_the_box_and_scales_with_the_window() {
        let b = Rect { min: Vec2::new(100.0, 50.0), max: Vec2::new(180.0, 130.0) };
        let at_design = row_centres(b, DESIGN_HEIGHT);
        assert_eq!(at_design[2].x, 140.0, "the middle icon is on the box's centre line");
        assert_eq!(at_design[2].y, 130.0 + BELOW_BOX);
        assert_eq!(at_design[1].x - at_design[0].x, PITCH);
        assert_eq!(at_design[0].x + at_design[4].x, 280.0, "symmetric about the centre");
        // At twice the window height the spacing and the offset below the box double.
        let double = row_centres(b, DESIGN_HEIGHT * 2.0);
        assert_eq!(double[1].x - double[0].x, PITCH * 2.0);
        assert_eq!(double[2].y, 130.0 + BELOW_BOX * 2.0);
    }

    #[test]
    fn a_system_is_offline_damaged_or_fine() {
        assert_eq!(look_of(&sys(false, 10, 10)), Look::Offline, "switched off, whatever its health");
        assert_eq!(look_of(&sys(true, 0, 10)), Look::Offline, "destroyed");
        assert_eq!(look_of(&sys(true, 5, 10)), Look::Damaged);
        assert_eq!(look_of(&sys(true, 10, 10)), Look::Fine);
        let tints = [tint(Look::Offline), tint(Look::Damaged), tint(Look::Fine)];
        assert!(tints[0] != tints[1] && tints[1] != tints[2] && tints[0] != tints[2], "three different looks");
    }

    #[test]
    fn nodes_follow_what_is_wanted() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<Assets<Image>>();
        let art = RowArt { image: Handle::default(), rect: Rect::new(0.0, 0.0, 8.0, 8.0), size: UVec2::new(16, 16) };
        let id = |i: u8| SystemIcon { unit: EntityId(1), index: i };
        let run = |app: &mut App, want: BTreeMap<SystemIcon, Placed>| {
            let art = art.clone();
            app.world_mut().run_system_once(move |mut c: Commands, mut q: IconQuery| reconcile(&mut c, &mut q, &want, &art, Vec2::splat(8.0), 1.0)).unwrap();
            app.world_mut().flush();
        };
        let count = |app: &mut App| app.world_mut().query::<&SystemIcon>().iter(app.world()).count();
        let placed = |x: f32, look| Placed { centre: Vec2::new(x, 20.0), look };
        run(&mut app, BTreeMap::from([(id(0), placed(10.0, Look::Fine)), (id(1), placed(24.0, Look::Damaged))]));
        assert_eq!(count(&mut app), 2);
        run(&mut app, BTreeMap::from([(id(1), placed(30.0, Look::Offline))]));
        assert_eq!(count(&mut app), 1, "the other icon is gone");
        let mut q = app.world_mut().query::<(&Node, &ImageNode)>();
        let (n, i) = q.single(app.world()).unwrap();
        assert_eq!((n.left, n.top), (Val::Px(26.0), Val::Px(16.0)), "centre minus half the icon");
        assert_eq!(i.color, tint(Look::Offline));
        run(&mut app, BTreeMap::new());
        assert_eq!(count(&mut app), 0, "the pointer left: the row is gone");
    }
}
