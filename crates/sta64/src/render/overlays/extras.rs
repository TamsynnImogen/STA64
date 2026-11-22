//! Armada II readout extra (issue 1046, follow-up of the bars in [`super::readout`]): the control-group
//! number. Stock look only, through [`super::armada2_look`].
//!
//! Rules (grades as in [`super::readout`]: "static" is read from the Armada II executable):
//! - Shown for a unit that is hovered, or selected, whatever the hover fade [static]. A class with the
//!   ODF key `is_starbase` is skipped unless it is itself the hovered unit [static].
//! - Position: its top left corner sits one quarter of the box width right of the box's right edge and
//!   one quarter of its height above its top edge, in camera pixels [static].
//! - Colour: the digit's colour is a constant of the engine [static]; see [`NUMBER_COLOUR`].
//! - The number is the unit's control group, 1 to 9; a unit in no group shows none. Interim: a unit in
//!   several STA64 groups shows the first, and how the original's group indices map to the keys is not
//!   established.
//! - Interim: the font. The engine's default font is used by the original; which picture font that is
//!   and at what size is not identified, so STA64 draws the digit in the mod's sprite font.
//! - Omitted (marked): the sprite laid under the number, read through the key `group_background` with a
//!   tint [static]; it is not drawn.
//! - The damaged-system icons are not part of this module (not established, see `TODO.md`).

use std::collections::{BTreeMap, BTreeSet};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;
use bevy::ui::UiScale;
use sta_sim::EntityId;

use super::{DEFAULT_RADIUS, ModelBounds, armada2_look, geometry, readout};
use crate::input::hover::HoverReadout;
use crate::input::select::ControlGroups;
use crate::render::units::SelectRadius;
use crate::sim_link::{ContentRes, Selection, SimEntities, SimLink};
use crate::ui::profile::UiProfile;
use crate::ui::sprite_font::{SpriteText, SpriteTextStyle, sprite_text};

/// The text colour of the number [static].
pub const NUMBER_COLOUR: Color = Color::srgb(0.8, 0.7, 0.2);

/// The root of one group-number node, a UI node placed over the unit's projected box.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GroupNumber {
    pub unit: EntityId,
}

/// The control group (1 to 9) `id` is in, if any; the first when it is in several [interim].
pub fn group_number(groups: &ControlGroups, id: EntityId) -> Option<u8> {
    groups.groups.iter().position(|g| g.contains(&id)).map(|n| n as u8 + 1)
}

/// Whether unit `id` shows its group number: hovered, or selected and not a starbase class [static].
pub fn shows_group(readout: &HoverReadout, id: EntityId, selected: bool, starbase: bool) -> bool {
    readout.unit == Some(id) || (selected && !starbase)
}

/// Where the number's top left corner goes for a unit box [static].
pub fn group_anchor(b: Rect) -> Vec2 {
    Vec2::new(b.max.x + (b.width() / 4.0).trunc(), b.min.y - (b.height() / 4.0).trunc())
}

/// The units that show a number this frame, with the number.
pub fn decide(
    readout: &HoverReadout,
    selection: &[EntityId],
    groups: &ControlGroups,
    exists: &dyn Fn(EntityId) -> bool,
    starbase: &dyn Fn(EntityId) -> bool,
) -> BTreeMap<EntityId, u8> {
    let candidates: BTreeSet<EntityId> = selection.iter().copied().chain(readout.unit).collect();
    candidates
        .into_iter()
        .filter(|&id| exists(id) && shows_group(readout, id, selection.contains(&id), starbase(id)))
        .filter_map(|id| group_number(groups, id).map(|n| (id, n)))
        .collect()
}

/// One number's place on screen, in camera pixels, and its digit.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub pos: Vec2,
    pub text: String,
}

/// The nodes of the numbers.
pub type ExtraQuery<'w, 's> = Query<'w, 's, (Entity, &'static GroupNumber, &'static mut Node, &'static mut SpriteText)>;

/// Bring the UI nodes in line with `want`, keeping the ones that stay. `ui_scale` converts camera
/// pixels to UI pixels.
pub fn reconcile(commands: &mut Commands, existing: &mut ExtraQuery, want: &BTreeMap<GroupNumber, Placed>, ui_scale: f32) {
    let mut have = BTreeSet::new();
    for (e, x, mut node, mut text) in existing.iter_mut() {
        let Some(p) = want.get(x).filter(|_| have.insert(*x)) else {
            commands.entity(e).try_despawn();
            continue;
        };
        let (left, top) = (Val::Px(p.pos.x / ui_scale), Val::Px(p.pos.y / ui_scale));
        if node.left != left {
            node.left = left;
        }
        if node.top != top {
            node.top = top;
        }
        if text.text != p.text {
            text.text = p.text.clone();
        }
    }
    for (x, p) in want {
        if have.contains(x) {
            continue;
        }
        let node = Node { position_type: PositionType::Absolute, left: Val::Px(p.pos.x / ui_scale), top: Val::Px(p.pos.y / ui_scale), ..default() };
        commands.spawn((Name::new("group number"), *x, node, sprite_text(p.text.clone(), 16.0, NUMBER_COLOUR), SpriteTextStyle { native: true, ..default() }));
    }
}

/// The optional inputs of [`sync_extras`], in one parameter.
#[derive(SystemParam)]
pub(super) struct ExtraConfig<'w> {
    content: Option<Res<'w, ContentRes>>,
    cine: Option<Res<'w, crate::ui::presentation::CineractiveState>>,
    ui_scale: Option<Res<'w, UiScale>>,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn sync_extras(
    mut commands: Commands,
    profile: Option<Res<UiProfile>>,
    readout: Res<HoverReadout>,
    selection: Res<Selection>,
    groups: Option<Res<ControlGroups>>,
    cfg: ExtraConfig,
    link: Res<SimLink>,
    ents: Res<SimEntities>,
    cam: Query<(Entity, &Camera, &Projection), crate::map::MainCamera>,
    parents: Query<(Option<&ModelBounds>, Option<&SelectRadius>)>,
    helper: TransformHelper,
    mut existing: ExtraQuery,
) {
    let ExtraConfig { content, cine, ui_scale } = cfg;
    let mut want: BTreeMap<GroupNumber, Placed> = BTreeMap::new();
    let mut scale = 1.0;
    if armada2_look(profile.as_deref())
        && !crate::ui::presentation::cutscene_running(cine)
        && let Some(groups) = groups.as_deref()
        && let Ok((cam_entity, camera, projection)) = cam.single()
        && let (Some(size), Ok(transform)) = (camera.logical_viewport_size(), helper.compute_global_transform(cam_entity))
    {
        let origin = camera.logical_viewport_rect().map_or(Vec2::ZERO, |r| r.min);
        let clip = projection.get_clip_from_view() * transform.to_matrix().inverse();
        scale = ui_scale.map_or(1.0, |s| s.0).max(1e-3);
        let decided = decide(
            &readout,
            &selection.0,
            groups,
            &|id| link.curr().unit(id).is_some(),
            &|id| link.curr().unit(id).is_some_and(|u| readout::is_starbase(content.as_deref(), u.def)),
        );
        for (id, n) in decided {
            let Some(&root) = ents.0.get(&id) else { continue };
            let Ok(world) = helper.compute_global_transform(root) else { continue };
            let Ok((bounds, radius)) = parents.get(root) else { continue };
            let bounds = bounds.copied().unwrap_or_else(|| ModelBounds::fallback(radius.map_or(DEFAULT_RADIUS, |r| r.0)));
            let Some((b, _)) = geometry::project(bounds, world.to_matrix(), clip, size) else { continue };
            let b = Rect { min: b.min + origin, max: b.max + origin };
            want.insert(GroupNumber { unit: id }, Placed { pos: group_anchor(b), text: n.to_string() });
        }
    }
    reconcile(&mut commands, &mut existing, &want, scale);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use sta_sim::PlayerId;

    fn hov(unit: u32, fade: f32) -> HoverReadout {
        HoverReadout { unit: Some(EntityId(unit)), team: Some(PlayerId(1)), fade }
    }

    fn groups_with(n: usize, ids: &[u32]) -> ControlGroups {
        let mut g = ControlGroups::default();
        g.groups[n] = ids.iter().map(|i| EntityId(*i)).collect();
        g
    }

    const EXISTS: fn(EntityId) -> bool = |_| true;
    const NOT_STARBASE: fn(EntityId) -> bool = |_| false;

    #[test]
    fn the_group_number_is_the_units_group() {
        let g = groups_with(2, &[7, 8]);
        assert_eq!(group_number(&g, EntityId(7)), Some(3));
        assert_eq!(group_number(&g, EntityId(9)), None);
        // In several groups: the first.
        let mut g = groups_with(2, &[7]);
        g.groups[4] = vec![EntityId(7)];
        assert_eq!(group_number(&g, EntityId(7)), Some(3));
    }

    #[test]
    fn group_number_shows_for_selected_or_hovered_regardless_of_the_fade() {
        let g = groups_with(0, &[1, 2, 3]);
        // Selected, nothing hovered, fade 0: still shown.
        let out = decide(&HoverReadout::default(), &[EntityId(1)], &g, &EXISTS, &NOT_STARBASE);
        assert_eq!(out.get(&EntityId(1)), Some(&1));
        // Hovered, not selected, fade 0.1: shown.
        let out = decide(&hov(2, 0.1), &[], &g, &EXISTS, &NOT_STARBASE);
        assert_eq!(out.get(&EntityId(2)), Some(&1));
        // Neither selected nor hovered (unit 3 is in the group): absent.
        let out = decide(&hov(2, 1.0), &[EntityId(1)], &g, &EXISTS, &NOT_STARBASE);
        assert!(!out.contains_key(&EntityId(3)));
        // Selected or hovered but in no group: absent.
        assert!(decide(&hov(5, 1.0), &[EntityId(4)], &g, &EXISTS, &NOT_STARBASE).is_empty());
    }

    #[test]
    fn a_starbase_class_shows_its_group_only_when_hovered() {
        let g = groups_with(0, &[1]);
        let all = |_: EntityId| true;
        assert!(decide(&HoverReadout::default(), &[EntityId(1)], &g, &EXISTS, &all).is_empty(), "selected starbase");
        assert_eq!(decide(&hov(1, 0.0), &[EntityId(1)], &g, &EXISTS, &all).len(), 1, "hovered starbase");
    }

    #[test]
    fn the_anchor_follows_the_box() {
        let b = Rect { min: Vec2::new(100.0, 50.0), max: Vec2::new(180.0, 130.0) };
        assert_eq!(group_anchor(b), Vec2::new(200.0, 30.0));
    }

    #[test]
    fn nodes_follow_what_is_wanted() {
        let mut app = App::new();
        let one = GroupNumber { unit: EntityId(1) };
        let two = GroupNumber { unit: EntityId(2) };
        let placed = |text: &str, x: f32| Placed { pos: Vec2::new(x, 10.0), text: text.into() };
        let run = |app: &mut App, want: BTreeMap<GroupNumber, Placed>| {
            app.world_mut().run_system_once(move |mut c: Commands, mut q: ExtraQuery| reconcile(&mut c, &mut q, &want, 1.0)).unwrap();
            app.world_mut().flush();
        };
        let count = |app: &mut App| app.world_mut().query::<&GroupNumber>().iter(app.world()).count();
        run(&mut app, BTreeMap::from([(one, placed("3", 5.0)), (two, placed("2", 6.0))]));
        assert_eq!(count(&mut app), 2);
        run(&mut app, BTreeMap::from([(one, placed("4", 7.0))]));
        assert_eq!(count(&mut app), 1, "the other number is gone, this one stays");
        let mut q = app.world_mut().query::<(&Node, &SpriteText)>();
        let (n, t) = q.single(app.world()).unwrap();
        assert_eq!((t.text.as_str(), n.left), ("4", Val::Px(7.0)));
        run(&mut app, BTreeMap::new());
        assert_eq!(count(&mut app), 0);
    }
}
