//! Armada II readout extras (issue 1046, follow-up of the bars in [`super::readout`]): the control-group
//! number and the damaged-system icons. Stock look only, through [`super::armada2_look`].
//!
//! Rules (grades as in [`super::readout`]: "static" is read from the Armada II executable, "data" from
//! the install's files):
//! - Group number: shown for a unit that is hovered, or selected, whatever the hover fade [static]. A
//!   class with the ODF key `is_starbase` is skipped unless it is itself the hovered unit [static]. The
//!   number is the unit's control group; a unit in no group shows none. Its top left corner sits one
//!   quarter of the box width right of the box's right edge and one quarter of its height above its
//!   top edge [static, in camera pixels]. Interim: the text is drawn in the mod's sprite font without
//!   the sprite-table background the original lays under it [not drawn], and in a fixed colour.
//! - System icons: shown under exactly the conditions of the bars (`readout::shows`: hover
//!   fade above 0.5, hovered or selected on the hovered unit's team) [static], one for each system
//!   that is damaged (below full hit points) or switched off [interim: the original's own test
//!   was not read]. Their top edge is 30 px plus the bar height below the box bottom [static]. Their
//!   intensity follows the fade past 0.5 [static]. Interim: the horizontal anchor is the box centre
//!   and the icon's top left corner sits on it, offset by the cfg rectangle's origin [not read].
//! - Pictures: the sprite named by the cfg key `selectSystem`, frame `n` for system `n`, drawn at the size
//!   of the cfg rectangle `selectSystem_<n>` [read from data]. Without the key or the rectangle, no icon.
//!   Frame `n` is the system of the sprite sheet's own frame labels (shields, engines, weapons, life
//!   support, sensors) [read from data, the labels being comments; the mapping is [`SELECT_SYSTEM`]].
//!   Interim: the icons are drawn in the order of those frames.
//! - Not drawn: the original's per-icon colouring by damage state.

use std::collections::{BTreeMap, BTreeSet};

use bevy::ecs::system::SystemParam;
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;
use bevy::ui::UiScale;
use sta_sim::combat::damage::SystemView;
use sta_sim::{EntityId, Fixed};

use super::{DEFAULT_RADIUS, GaugeStyle, ModelBounds, armada2_look, geometry, readout};
use crate::input::hover::HoverReadout;
use crate::input::select::ControlGroups;
use crate::render::units::SelectRadius;
use crate::sim_link::{ContentRes, Selection, SimEntities, SimLink};
use crate::ui::gui_sprites::GuiImage;
use crate::ui::hud::{HudConfig, StatusFeed};
use crate::ui::profile::UiProfile;
use crate::ui::sprite_font::{SpriteText, SpriteTextStyle, sprite_text};

/// Pixels from the box bottom to the icons' top edge, before the bar height is added [static].
pub const ICON_DROP: f32 = 30.0;

/// The simulation's system index ([`SystemView`] order: engines, weapons, shield generator, life
/// support, sensors) shown by frame `n` of the `selectSystem` sprite [read from data].
pub const SELECT_SYSTEM: [usize; 5] = [2, 0, 1, 3, 4];

/// What one extra shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExtraKind {
    /// The control-group number.
    Group,
    /// The damaged-system icon of sprite frame `n`.
    System(u8),
}

/// The root of one extra, a UI node placed over the unit's projected box.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Extra {
    pub unit: EntityId,
    pub kind: ExtraKind,
}

/// The control group (1 to 9) `id` is in, if any.
pub fn group_number(groups: &ControlGroups, id: EntityId) -> Option<u8> {
    groups.groups.iter().position(|g| g.contains(&id)).map(|n| n as u8 + 1)
}

/// Whether unit `id` shows its group number: hovered, or selected and not a starbase class [static].
pub fn shows_group(readout: &HoverReadout, id: EntityId, selected: bool, starbase: bool) -> bool {
    readout.unit == Some(id) || (selected && !starbase)
}

/// The sprite frames to draw for `v`: one per damaged or switched-off system [interim].
pub fn damaged_frames(v: &SystemView) -> Vec<u8> {
    (0..SELECT_SYSTEM.len())
        .filter(|&n| {
            let s = SELECT_SYSTEM[n];
            v.health[s] < Fixed::ONE || !v.on[s]
        })
        .map(|n| n as u8)
        .collect()
}

/// Where the group number's top left corner goes for a unit box [static].
pub fn group_anchor(b: Rect) -> Vec2 {
    Vec2::new(b.max.x + (b.width() / 4.0).trunc(), b.min.y - (b.height() / 4.0).trunc())
}

/// The icons' anchor for a unit box and bar height: horizontal anchor interim, height [static].
pub fn icon_anchor(b: Rect, bar_height: f32) -> Vec2 {
    Vec2::new(b.center().x, b.max.y + ICON_DROP + bar_height)
}

/// The brightness of an icon at hover fade `f` [static].
pub fn icon_intensity(f: f32) -> f32 {
    (f - HoverReadout::BARS_ABOVE).clamp(0.0, 1.0)
}

/// What to show for the units of one frame, from plain inputs.
pub fn decide(
    readout: &HoverReadout,
    selection: &[EntityId],
    groups: &ControlGroups,
    systems: &[SystemView],
    owner_of: &dyn Fn(EntityId) -> Option<sta_sim::PlayerId>,
    starbase: &dyn Fn(EntityId) -> bool,
) -> BTreeSet<Extra> {
    let mut out = BTreeSet::new();
    let candidates: BTreeSet<EntityId> = selection.iter().copied().chain(readout.unit).collect();
    for id in candidates {
        let Some(owner) = owner_of(id) else { continue };
        let selected = selection.contains(&id);
        let sb = starbase(id);
        if shows_group(readout, id, selected, sb) && group_number(groups, id).is_some() {
            out.insert(Extra { unit: id, kind: ExtraKind::Group });
        }
        if readout::shows(readout, id, owner, selected, sb)
            && let Some(v) = systems.iter().find(|v| v.entity == id)
        {
            out.extend(damaged_frames(v).into_iter().map(|n| Extra { unit: id, kind: ExtraKind::System(n) }));
        }
    }
    out
}

/// One extra's place on screen, in UI pixels, and what it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub pos: Vec2,
    pub size: Vec2,
    pub text: Option<String>,
    pub frame: usize,
    pub sprite: String,
    pub intensity: f32,
}

/// The picture of the icons: the sprite name and the rectangle of frame `n`, from the cfg [read from data].
fn icon_art(gui: &sta_content::GuiLayout, n: u8) -> Option<(String, sta_content::Rect)> {
    let name = gui.names.iter().find(|(k, _)| k.eq_ignore_ascii_case("selectSystem"))?.1.clone();
    let key = format!("selectSystem_{n}");
    let rect = *gui.rects.iter().find(|(k, _)| k.eq_ignore_ascii_case(&key))?.1;
    (rect.w > 0 && rect.h > 0).then_some((name, rect))
}

/// Place `x` over the unit box `b` (camera pixels).
pub fn place(x: Extra, b: Rect, group: Option<u8>, gui: Option<&sta_content::GuiLayout>, bar_height: f32, fade: f32) -> Option<Placed> {
    match x.kind {
        ExtraKind::Group => Some(Placed {
            pos: group_anchor(b),
            size: Vec2::ZERO,
            text: Some(group?.to_string()),
            frame: 0,
            sprite: String::new(),
            intensity: 1.0,
        }),
        ExtraKind::System(n) => {
            let (sprite, r) = icon_art(gui?, n)?;
            let a = icon_anchor(b, bar_height);
            Some(Placed { pos: a + Vec2::new(r.x as f32, r.y as f32), size: Vec2::new(r.w as f32, r.h as f32), text: None, frame: n as usize, sprite, intensity: icon_intensity(fade) })
        }
    }
}

/// The text colour of the number [interim].
const NUMBER_COLOUR: Color = Color::srgb(0.8, 0.7, 0.2);

/// The nodes of the extras.
pub type ExtraQuery<'w, 's> = Query<'w, 's, (Entity, &'static Extra, &'static mut Node, Option<&'static mut SpriteText>, Option<&'static mut GuiImage>)>;

/// Bring the UI nodes in line with `want`, keeping the ones that stay.
pub fn reconcile(
    commands: &mut Commands,
    existing: &mut ExtraQuery,
    want: &BTreeMap<Extra, Placed>,
    ui_scale: f32,
) {
    let mut have = BTreeSet::new();
    for (e, x, mut node, text, image) in existing.iter_mut() {
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
        if let (Some(mut t), Some(s)) = (text, &p.text)
            && &t.text != s
        {
            t.text = s.clone();
        }
        if let Some(mut i) = image {
            let c = Color::srgba(1.0, 1.0, 1.0, p.intensity);
            if i.color != c {
                i.color = c;
            }
        }
    }
    for (x, p) in want {
        if have.contains(x) {
            continue;
        }
        let node = Node { position_type: PositionType::Absolute, left: Val::Px(p.pos.x / ui_scale), top: Val::Px(p.pos.y / ui_scale), width: if p.size.x > 0.0 { Val::Px(p.size.x / ui_scale) } else { Val::Auto }, height: if p.size.y > 0.0 { Val::Px(p.size.y / ui_scale) } else { Val::Auto }, ..default() };
        match &p.text {
            Some(t) => {
                commands.spawn((Name::new("group number"), *x, node, sprite_text(t.clone(), 16.0, NUMBER_COLOUR), SpriteTextStyle { native: true, ..default() }));
            }
            None => {
                commands.spawn((Name::new("system icon"), *x, node, Pickable::IGNORE, GuiImage::new(p.sprite.clone()).with_frame(p.frame).with_color(Color::srgba(1.0, 1.0, 1.0, p.intensity))));
            }
        }
    }
}

/// The optional inputs of [`sync_extras`], in one parameter.
#[derive(SystemParam)]
pub(super) struct ExtraConfig<'w> {
    status: Option<Res<'w, StatusFeed>>,
    hud: Option<Res<'w, HudConfig>>,
    content: Option<Res<'w, ContentRes>>,
    cine: Option<Res<'w, crate::ui::presentation::CineractiveState>>,
    style: Res<'w, GaugeStyle>,
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
    let ExtraConfig { status, hud, content, cine, style, ui_scale } = cfg;
    let mut want: BTreeMap<Extra, Placed> = BTreeMap::new();
    if armada2_look(profile.as_deref())
        && !crate::ui::presentation::cutscene_running(cine)
        && let Some(groups) = groups.as_deref()
        && let Ok((cam_entity, camera, projection)) = cam.single()
        && let (Some(size), Ok(transform)) = (camera.logical_viewport_size(), helper.compute_global_transform(cam_entity))
    {
        let origin = camera.logical_viewport_rect().map_or(Vec2::ZERO, |r| r.min);
        let clip = projection.get_clip_from_view() * transform.to_matrix().inverse();
        let systems: &[SystemView] = status.as_deref().map_or(&[], |s| &s.systems);
        let decided = decide(
            &readout,
            &selection.0,
            groups,
            systems,
            &|id| link.curr().unit(id).map(|u| u.owner),
            &|id| link.curr().unit(id).is_some_and(|u| readout::is_starbase(content.as_deref(), u.def)),
        );
        let scale = ui_scale.map_or(1.0, |s| s.0).max(1e-3);
        for x in decided {
            let Some(&root) = ents.0.get(&x.unit) else { continue };
            let Ok(world) = helper.compute_global_transform(root) else { continue };
            let Ok((bounds, radius)) = parents.get(root) else { continue };
            let bounds = bounds.copied().unwrap_or_else(|| ModelBounds::fallback(radius.map_or(DEFAULT_RADIUS, |r| r.0)));
            let Some((b, _)) = geometry::project(bounds, world.to_matrix(), clip, size) else { continue };
            let b = Rect { min: b.min + origin, max: b.max + origin };
            if let Some(p) = place(x, b, group_number(groups, x.unit), hud.as_deref().and_then(|h| h.gui.as_ref()), style.height, readout.fade) {
                want.insert(x, p);
            }
        }
        // Positions are camera pixels; the UI works in its own scaled pixels.
        reconcile(&mut commands, &mut existing, &want, scale);
        return;
    }
    reconcile(&mut commands, &mut existing, &want, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use sta_sim::PlayerId;

    fn hov(unit: u32, team: u8, fade: f32) -> HoverReadout {
        HoverReadout { unit: Some(EntityId(unit)), team: Some(PlayerId(team)), fade }
    }

    fn groups_with(n: usize, ids: &[u32]) -> ControlGroups {
        let mut g = ControlGroups::default();
        g.groups[n] = ids.iter().map(|i| EntityId(*i)).collect();
        g
    }

    fn sys(id: u32, health: [f64; 5], on: [bool; 5]) -> SystemView {
        SystemView { entity: EntityId(id), health: health.map(Fixed::from_f64), on }
    }

    const ME: fn(EntityId) -> Option<PlayerId> = |_| Some(PlayerId(1));
    const NOT_STARBASE: fn(EntityId) -> bool = |_| false;

    fn kinds(set: &BTreeSet<Extra>, unit: u32) -> Vec<ExtraKind> {
        set.iter().filter(|x| x.unit == EntityId(unit)).map(|x| x.kind).collect()
    }

    #[test]
    fn the_group_number_is_the_units_group() {
        let g = groups_with(2, &[7, 8]);
        assert_eq!(group_number(&g, EntityId(7)), Some(3));
        assert_eq!(group_number(&g, EntityId(9)), None);
    }

    #[test]
    fn group_number_shows_for_selected_or_hovered_regardless_of_the_fade() {
        let g = groups_with(0, &[1, 2, 3]);
        // Selected, nothing hovered, fade 0: still shown.
        let out = decide(&HoverReadout::default(), &[EntityId(1)], &g, &[], &ME, &NOT_STARBASE);
        assert_eq!(kinds(&out, 1), vec![ExtraKind::Group]);
        // Hovered, not selected, fade 0.1: shown.
        let out = decide(&hov(2, 1, 0.1), &[], &g, &[], &ME, &NOT_STARBASE);
        assert_eq!(kinds(&out, 2), vec![ExtraKind::Group]);
        // Neither selected nor hovered (unit 3 is in the group): absent.
        let out = decide(&hov(2, 1, 1.0), &[EntityId(1)], &g, &[], &ME, &NOT_STARBASE);
        assert!(kinds(&out, 3).is_empty());
        // Selected or hovered but in no group: absent.
        let out = decide(&hov(5, 1, 1.0), &[EntityId(4)], &g, &[], &ME, &NOT_STARBASE);
        assert!(out.is_empty());
    }

    #[test]
    fn a_starbase_class_shows_its_group_only_when_hovered() {
        let g = groups_with(0, &[1]);
        let all = |_: EntityId| true;
        let out = decide(&HoverReadout::default(), &[EntityId(1)], &g, &[], &ME, &all);
        assert!(out.is_empty(), "selected starbase");
        let out = decide(&hov(1, 1, 0.0), &[EntityId(1)], &g, &[], &ME, &all);
        assert_eq!(kinds(&out, 1), vec![ExtraKind::Group], "hovered starbase");
    }

    #[test]
    fn icons_are_exactly_the_damaged_systems() {
        let whole = sys(1, [1.0; 5], [true; 5]);
        assert!(damaged_frames(&whole).is_empty());
        // Engines (sim 0) damaged, weapons (sim 1) switched off, sensors (sim 4) whole.
        let v = sys(1, [0.5, 1.0, 1.0, 1.0, 1.0], [true, false, true, true, true]);
        let frames = damaged_frames(&v);
        assert_eq!(frames.iter().map(|&n| SELECT_SYSTEM[n as usize]).collect::<Vec<_>>(), vec![0, 1]);
        // A destroyed system is damaged too.
        assert_eq!(damaged_frames(&sys(1, [1.0, 1.0, 1.0, 1.0, 0.0], [true; 5])).len(), 1);
        // The mapping covers every system once.
        let mut m = SELECT_SYSTEM.to_vec();
        m.sort();
        assert_eq!(m, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn icons_follow_the_bars_visibility_rule() {
        let g = ControlGroups::default();
        let hurt = [sys(1, [0.5, 1.0, 1.0, 1.0, 1.0], [true; 5]), sys(2, [0.5, 1.0, 1.0, 1.0, 1.0], [true; 5]), sys(3, [0.5, 1.0, 1.0, 1.0, 1.0], [true; 5])];
        let n = |out: &BTreeSet<Extra>, u| kinds(out, u).len();
        // Selection alone: none. Hover under the fade threshold: none.
        assert!(decide(&HoverReadout::default(), &[EntityId(1)], &g, &hurt, &ME, &NOT_STARBASE).is_empty());
        assert!(decide(&hov(2, 1, 0.5), &[EntityId(1)], &g, &hurt, &ME, &NOT_STARBASE).is_empty());
        // Past it: the hovered unit and the selected unit of its team.
        let out = decide(&hov(2, 1, 0.6), &[EntityId(1)], &g, &hurt, &ME, &NOT_STARBASE);
        assert_eq!((n(&out, 1), n(&out, 2), n(&out, 3)), (1, 1, 0));
        // A selected unit of another team than the hovered one: none for it.
        let team = |id: EntityId| Some(PlayerId(if id.0 == 1 { 2 } else { 1 }));
        let out = decide(&hov(2, 1, 0.9), &[EntityId(1)], &g, &hurt, &team, &NOT_STARBASE);
        assert_eq!((n(&out, 1), n(&out, 2)), (0, 1));
        // A whole unit has none; a starbase class only when hovered.
        let whole = [sys(2, [1.0; 5], [true; 5])];
        assert!(decide(&hov(2, 1, 1.0), &[], &g, &whole, &ME, &NOT_STARBASE).is_empty());
        let sb = |_: EntityId| true;
        assert_eq!(n(&decide(&hov(2, 1, 1.0), &[EntityId(1)], &g, &hurt, &ME, &sb), 1), 0);
    }

    #[test]
    fn anchors_follow_the_box() {
        let b = Rect { min: Vec2::new(100.0, 50.0), max: Vec2::new(180.0, 130.0) };
        assert_eq!(group_anchor(b), Vec2::new(200.0, 30.0));
        assert_eq!(icon_anchor(b, 2.0), Vec2::new(140.0, 162.0));
        assert_eq!(icon_intensity(0.5), 0.0);
        assert!((icon_intensity(1.0) - 0.5).abs() < 1e-6);
    }

    fn layout(text: &str) -> sta_content::GuiLayout {
        sta_content::GuiCfg::parse("t.cfg", text).0.layout_auto()
    }

    #[test]
    fn icon_art_is_read_from_the_cfg_keys_and_absent_without_them() {
        let b = Rect { min: Vec2::new(0.0, 0.0), max: Vec2::new(40.0, 40.0) };
        let x = Extra { unit: EntityId(1), kind: ExtraKind::System(2) };
        let l = layout("selectSystem = \"invented\"\nselectSystem_2 = 3 4 9 8\n");
        let p = place(x, b, None, Some(&l), 2.0, 1.0).unwrap();
        assert_eq!((p.sprite.as_str(), p.frame, p.size), ("invented", 2, Vec2::new(9.0, 8.0)));
        assert_eq!(p.pos, Vec2::new(20.0 + 3.0, 40.0 + 32.0 + 4.0));
        assert!(place(x, b, None, Some(&layout("selectSystem_2 = 3 4 9 8\n")), 2.0, 1.0).is_none(), "no sprite key");
        assert!(place(x, b, None, Some(&layout("selectSystem = \"invented\"\n")), 2.0, 1.0).is_none(), "no rectangle");
        assert!(place(x, b, None, None, 2.0, 1.0).is_none(), "no layout");
        let g = place(Extra { unit: EntityId(1), kind: ExtraKind::Group }, b, Some(4), None, 2.0, 1.0).unwrap();
        assert_eq!(g.text.as_deref(), Some("4"));
    }

    #[test]
    fn nodes_follow_what_is_wanted() {
        let mut app = App::new();
        let group = Extra { unit: EntityId(1), kind: ExtraKind::Group };
        let icon = Extra { unit: EntityId(1), kind: ExtraKind::System(0) };
        let placed = |text: Option<&str>, x: f32| Placed { pos: Vec2::new(x, 10.0), size: if text.is_some() { Vec2::ZERO } else { Vec2::new(8.0, 8.0) }, text: text.map(str::to_owned), frame: 0, sprite: "invented".into(), intensity: 0.5 };
        let run = |app: &mut App, want: BTreeMap<Extra, Placed>| {
            app.world_mut()
                .run_system_once(move |mut c: Commands, mut q: ExtraQuery| reconcile(&mut c, &mut q, &want, 1.0))
                .unwrap();
            app.world_mut().flush();
        };
        let count = |app: &mut App| app.world_mut().query::<&Extra>().iter(app.world()).count();
        run(&mut app, BTreeMap::from([(group, placed(Some("3"), 5.0)), (icon, placed(None, 6.0))]));
        assert_eq!(count(&mut app), 2);
        run(&mut app, BTreeMap::from([(group, placed(Some("4"), 7.0))]));
        assert_eq!(count(&mut app), 1, "the icon is gone, the number stays");
        let mut q = app.world_mut().query::<(&Node, &SpriteText)>();
        let (n, t) = q.single(app.world()).unwrap();
        assert_eq!((t.text.as_str(), n.left), ("4", Val::Px(7.0)));
        run(&mut app, BTreeMap::new());
        assert_eq!(count(&mut app), 0);
    }
}
