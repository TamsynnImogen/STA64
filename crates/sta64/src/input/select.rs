//! Selection rules, from Refit's `qol/README.md` (QOL-3, 4, 5; stock behaviour
//! where it says nothing). The click rules are the usual RTS ones.
//!
//! - Left click selects the unit under the cursor (strategic icon first, else
//!   the nearest sphere on the ray). Click on empty space clears.
//! - Shift (the `AddModifier` key) click adds an unselected unit, removes a
//!   selected one.
//! - Left drag (over 6 px) box-selects own units whose centre is in the box;
//!   Shift keeps the old selection.
//! - Double click an own unit (same unit, within 0.4 s): all own units of the
//!   same def (class) that are on screen ("everything of that type on screen").
//! - `SelectType` (T): all own units of the selected units' classes, anywhere.
//! - The selection holds at most [`MaxSelection`] units (QOL-3: default 40,
//!   17 to 120); box, double click, select type and recall stop at the cap.
//!   A stored group has no limit of its own.
//! - Control groups 1..9: `N` selects the group (dead units drop out); a
//!   second `N` within 750 ms centres the camera on the group, on the member
//!   nearest the group's mean ("the centre of the bulk"). `Alt+N` selects and
//!   centres at once (stock's Shift+N). `Ctrl+N` replaces the group with the
//!   selection (empty selection empties it). `Shift+N` and `Ctrl+Shift+N` add
//!   the selection and keep what is there (QOL-4); with nothing selected they
//!   do nothing. A unit is in one group at a time: binding takes it out of
//!   every other group.
//! - Only own units are ever box- or group-selected. A single enemy unit may
//!   be selected alone (inspection); it is replaced, never joined, so orders
//!   can use [`own_selection`] and never see it.
//!
//! Not yet: stations in groups (QOL-5: one station class per group, ships and
//! stations never mixed), which needs the sim view to say what a station is.
//!
//! The result goes to [`Selection`]; nothing is marked on the entities. All
//! keys come from [`KeyBindings`]. Tests (and later UI-02's `Aim`) inject the
//! cursor, ray and projection through [`Aim`].

use bevy::prelude::*;
use sta_sim::{ContentKey, EntityId};

use super::orders::Aim;
use super::keys::{Action, KeyBindings};
use crate::render::strategic::Strategic;
use crate::render::units::SelectRadius;
use crate::sim_link::{Selection, SelectionChanged, SimEntity, SimLink};

/// Seconds within which a second click counts as a double click.
const DOUBLE: f64 = 0.4;
/// Seconds within which a second group tap centres the camera (stock: 750 ms).
const GROUP_TAP: f64 = 0.75;
/// Pixels of drag before a click becomes a box.
const DRAG_PX: f32 = 6.0;

/// The drag rectangle shown while box-selecting.
#[derive(Component)]
pub struct SelectBox;

#[derive(Resource, Default)]
pub struct Drag {
    start: Option<Vec2>,
    last_click: Option<(f64, EntityId)>,
}

/// Most units a selection can hold (Refit's `MaxSelection=`: default 40,
/// 17 to 120).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxSelection(usize);

#[allow(dead_code)] // the settings package sets it
impl MaxSelection {
    pub fn new(n: usize) -> Self {
        MaxSelection(n.clamp(17, 120))
    }

    pub fn get(self) -> usize {
        self.0
    }
}

impl Default for MaxSelection {
    fn default() -> Self {
        MaxSelection(40)
    }
}

/// Keep the first `max` units.
fn cap(v: &mut Vec<EntityId>, max: MaxSelection) {
    v.truncate(max.0);
}

/// Control groups 1..=9 and the last recall, for the double tap.
#[derive(Resource, Default, Debug)]
pub struct ControlGroups {
    pub groups: [Vec<EntityId>; 9],
    last_tap: Option<(f64, usize)>,
}

/// Ask the camera to centre on this map-space point (a double-tapped group).
/// The camera code reads it; selection does not move the camera.
#[derive(Message, Debug, Clone, Copy)]
#[allow(dead_code)] // read by the camera (not yet)
pub struct CameraFocus(pub Vec3);

/// The cursor, its ray and the camera's projection, this frame.
#[derive(Clone, Copy, Debug)]
pub struct AimState {
    pub cursor: Vec2,
    pub ray: Ray3d,
    /// World to clip space, to find where a unit is on screen.
    pub clip_from_world: Mat4,
    /// Viewport size in pixels.
    pub size: Vec2,
}

impl AimState {
    /// Where `p` is on screen, if it is in front of the camera.
    pub fn project(&self, p: Vec3) -> Option<Vec2> {
        let c = self.clip_from_world * p.extend(1.0);
        if c.w <= 0.0 {
            return None;
        }
        let n = c.truncate() / c.w;
        if !(0.0..=1.0).contains(&n.z) {
            return None;
        }
        Some(Vec2::new((n.x + 1.0) * 0.5 * self.size.x, (1.0 - n.y) * 0.5 * self.size.y))
    }

    fn on_screen(&self, p: Vec3) -> bool {
        self.project(p).is_some_and(|s| s.cmpge(Vec2::ZERO).all() && s.cmple(self.size).all())
    }
}

pub fn spawn_select_box(mut commands: Commands) {
    commands.spawn((
        SelectBox,
        Node { position_type: PositionType::Absolute, ..default() },
        BackgroundColor(Color::srgba(0.3, 1.0, 0.4, 0.15)),
        Visibility::Hidden,
    ));
}

/// The selected units this client may give orders to (the selection minus
/// any inspected enemy).
pub fn own_selection(selection: &Selection, link: &SimLink) -> Vec<EntityId> {
    selection.0.iter().copied().filter(|&id| is_own(link, id)).collect()
}

fn is_own(link: &SimLink, id: EntityId) -> bool {
    link.curr().unit(id).is_some_and(|u| u.owner == link.player())
}

fn def_of(link: &SimLink, id: EntityId) -> Option<ContentKey> {
    link.curr().unit(id).map(|u| u.def)
}

pub(super) type Targetable<'w, 's> = Query<'w, 's, (&'static SimEntity, &'static SelectRadius, &'static GlobalTransform, Option<&'static Strategic>)>;

/// The unit under the cursor: a strategic icon, else the nearest sphere on the ray.
pub(super) fn unit_under(cursor: Vec2, ray: Ray3d, units: &Targetable) -> Option<EntityId> {
    let icon = units
        .iter()
        .filter_map(|(u, _, _, s)| {
            let s = s.filter(|s| s.shown > 0.5)?;
            let d = s.screen.distance(cursor);
            (d <= s.size_px * 0.6).then_some((d, u.0))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((_, id)) = icon {
        return Some(id);
    }
    let mut best: Option<(f32, EntityId)> = None;
    for (u, radius, gt, _) in units {
        let to = gt.translation() - ray.origin;
        let t = to.dot(*ray.direction);
        if t < 0.0 {
            continue;
        }
        let miss = (to - *ray.direction * t).length();
        if miss <= radius.0 && best.is_none_or(|(bt, _)| t < bt) {
            best = Some((t, u.0));
        }
    }
    best.map(|b| b.1)
}

fn push_unique(v: &mut Vec<EntityId>, id: EntityId) {
    if !v.contains(&id) {
        v.push(id);
    }
}

/// Keys: select-by-type and control groups.
#[allow(clippy::too_many_arguments)]
pub fn select_keys(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    time: Res<Time<Real>>,
    link: Res<SimLink>,
    max: Res<MaxSelection>,
    mut selection: ResMut<Selection>,
    mut groups: ResMut<ControlGroups>,
    mut changed: MessageWriter<SelectionChanged>,
    mut focus: MessageWriter<CameraFocus>,
) {
    let now = time.elapsed_secs_f64();
    let before = selection.0.clone();
    if bindings.just_pressed(Action::SelectType, &keys) {
        let defs: Vec<ContentKey> = own_selection(&selection, &link).into_iter().filter_map(|id| def_of(&link, id)).collect();
        if !defs.is_empty() {
            selection.0 = link
                .curr()
                .units
                .iter()
                .filter(|u| u.owner == link.player() && defs.contains(&u.def))
                .map(|u| u.id)
                .collect();
            cap(&mut selection.0, *max);
        }
    }
    if let Some(n) = (0..9).find(|&n| bindings.just_pressed(Action::Group(n as u8), &keys)) {
        let ctrl = bindings.pressed(Action::GroupModifier, &keys);
        let add = bindings.pressed(Action::AddModifier, &keys);
        let alt = bindings.pressed(Action::CentreModifier, &keys);
        if ctrl || (add && !alt) {
            bind_group(&mut groups, n, own_selection(&selection, &link), add);
        } else {
            let alive: Vec<EntityId> = groups.groups[n].iter().copied().filter(|&id| is_own(&link, id)).collect();
            groups.groups[n] = alive.clone();
            if !alive.is_empty() {
                let double = groups.last_tap.is_some_and(|(t, g)| g == n && now - t <= GROUP_TAP);
                if double || alt {
                    if let Some(c) = bulk_centre(&link, &alive) {
                        focus.write(CameraFocus(c));
                    }
                    groups.last_tap = None;
                } else {
                    groups.last_tap = Some((now, n));
                }
                selection.0 = alive;
                cap(&mut selection.0, *max);
            }
        }
    }
    if selection.0 != before {
        changed.write(SelectionChanged);
    }
}

/// Store `ids` in group `n`: replacing it, or (`add`) joining what it holds.
/// Adding nothing keeps the group. Every id leaves the other groups.
fn bind_group(groups: &mut ControlGroups, n: usize, ids: Vec<EntityId>, add: bool) {
    if add && ids.is_empty() {
        return;
    }
    let mut g = if add { std::mem::take(&mut groups.groups[n]) } else { vec![] };
    for &id in &ids {
        push_unique(&mut g, id);
    }
    for (i, other) in groups.groups.iter_mut().enumerate() {
        if i != n {
            other.retain(|id| !ids.contains(id));
        }
    }
    groups.groups[n] = g;
}

/// Map-space position of the group member nearest the members' mean: the
/// "centre of the bulk" the camera points at.
fn bulk_centre(link: &SimLink, ids: &[EntityId]) -> Option<Vec3> {
    let ps: Vec<Vec3> = ids.iter().filter_map(|&id| link.curr().unit(id)).map(|u| Vec3::from(u.pos.to_f32())).collect();
    if ps.is_empty() {
        return None;
    }
    let mean = ps.iter().copied().sum::<Vec3>() / ps.len() as f32;
    let c = ps.into_iter().min_by(|a, b| a.distance_squared(mean).total_cmp(&b.distance_squared(mean)))?;
    // Sim to map space: z is mirrored.
    Some(Vec3::new(c.x, c.y, -c.z))
}


/// Mouse: click, shift-click, box, double click.
#[allow(clippy::too_many_arguments)]
pub fn select(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    time: Res<Time<Real>>,
    aim: Res<Aim>,
    link: Res<SimLink>,
    max: Res<MaxSelection>,
    units: Targetable,
    mut selection: ResMut<Selection>,
    mut changed: MessageWriter<SelectionChanged>,
    mut drag: ResMut<Drag>,
    mut boxes: Query<(&mut Node, &mut Visibility), With<SelectBox>>,
) {
    let Some(aim) = aim.0 else { return };
    let cursor = aim.cursor;
    if buttons.just_pressed(MouseButton::Left) {
        drag.start = Some(cursor);
    }
    let Some(start) = drag.start else { return };
    let dragging = start.distance(cursor) > DRAG_PX;
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
    let now = time.elapsed_secs_f64();
    let add = bindings.pressed(Action::AddModifier, &keys);
    let before = selection.0.clone();
    // An inspected enemy is never joined by anything.
    let mut sel: Vec<EntityId> = if add { own_selection(&selection, &link) } else { vec![] };
    if dragging {
        let (lo, hi) = (start.min(cursor), start.max(cursor));
        for (u, _, gt, _) in &units {
            if is_own(&link, u.0)
                && let Some(p) = aim.project(gt.translation())
                && p.cmpge(lo).all()
                && p.cmple(hi).all()
            {
                push_unique(&mut sel, u.0);
            }
        }
    } else if let Some(id) = unit_under(cursor, aim.ray, &units) {
        let double = drag.last_click.is_some_and(|(t, last)| last == id && now - t <= DOUBLE);
        drag.last_click = if double { None } else { Some((now, id)) };
        if !is_own(&link, id) {
            // Inspection only: alone, and not with Shift.
            if !add {
                sel = vec![id];
            }
        } else if double {
            let def = def_of(&link, id);
            for (u, _, gt, _) in &units {
                if is_own(&link, u.0) && def_of(&link, u.0) == def && aim.on_screen(gt.translation()) {
                    push_unique(&mut sel, u.0);
                }
            }
            push_unique(&mut sel, id);
        } else if add {
            // Toggle against the old selection.
            if let Some(i) = sel.iter().position(|&s| s == id) {
                sel.remove(i);
            } else {
                sel.push(id);
            }
        } else {
            sel = vec![id];
        }
    }
    cap(&mut sel, *max);
    selection.0 = sel;
    if selection.0 != before {
        changed.write(SelectionChanged);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use sta_sim::command::LocalRouter;
    use sta_sim::command::Command;
    use sta_sim::view::UnitFlags;
    use sta_sim::{Fixed, PlayerId, TickOutput, UnitView, Vec3 as SimVec3, WorldSource, WorldView};

    struct Frozen(Arc<WorldView>);

    impl WorldSource for Frozen {
        fn step(&mut self, _: &[Command]) -> TickOutput {
            TickOutput { view: self.0.clone(), events: vec![] }
        }
        fn view(&self) -> Arc<WorldView> {
            self.0.clone()
        }
    }

    fn unit(id: u32, def: u32, owner: u8, x: f32, z: f32) -> UnitView {
        UnitView {
            id: EntityId(id),
            def: ContentKey(def),
            owner: PlayerId(owner),
            pos: SimVec3::from_f32([x, 0.0, -z]),
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2],
            shields: [[Fixed::ONE; 2]; 4],
            flags: UnitFlags::default(),
            progress: None,
        }
    }

    const A: EntityId = EntityId(1);
    const B: EntityId = EntityId(2);
    const C: EntityId = EntityId(3);
    const E: EntityId = EntityId(4);
    const F: EntityId = EntityId(5);
    const D: EntityId = EntityId(6);

    /// Player 0 owns A, B (class 1), C (class 2), F (class 1, off screen) and
    /// D (class 1, on screen, far below); player 1 owns E (class 1). Render
    /// x/z are screen pixels in an 800x600 top-down view.
    fn app() -> App {
        let units = vec![
            unit(1, 1, 0, 100.0, 100.0),
            unit(2, 1, 0, 300.0, 100.0),
            unit(3, 2, 0, 500.0, 100.0),
            unit(4, 1, 1, 100.0, 300.0),
            unit(5, 1, 0, 5000.0, 100.0),
            unit(6, 1, 0, 700.0, 500.0),
        ];
        let view = Arc::new(WorldView { units, ..default() });
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<SelectionChanged>()
            .add_message::<CameraFocus>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Selection>()
            .init_resource::<Drag>()
            .init_resource::<ControlGroups>()
            .init_resource::<KeyBindings>()
            .init_resource::<MaxSelection>()
            .init_resource::<Aim>()
            .insert_resource(SimLink::new(Box::new(Frozen(view.clone())), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_systems(Update, (select_keys, select).chain());
        for u in &view.units {
            let [x, y, z] = u.pos.to_f32();
            app.world_mut().spawn((SimEntity(u.id), SelectRadius(50.0), GlobalTransform::from(Transform::from_xyz(x, y, -z))));
        }
        app
    }

    fn aim_at(app: &mut App, x: f32, z: f32) {
        // Screen (x, y) is world (x, z); depth is world y.
        let swap = Mat4::from_cols(Vec4::X, Vec4::Z, Vec4::Y, Vec4::W);
        let clip = Mat4::from_translation(Vec3::new(-1.0, 1.0, 0.5)) * Mat4::from_scale(Vec3::new(2.0 / 800.0, -2.0 / 600.0, 0.0)) * swap;
        app.world_mut().resource_mut::<Aim>().0 = Some(AimState {
            cursor: Vec2::new(x, z),
            ray: Ray3d::new(Vec3::new(x, 1000.0, z), Dir3::NEG_Y),
            clip_from_world: clip,
            size: Vec2::new(800.0, 600.0),
        });
    }

    fn step(app: &mut App) {
        app.update();
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().clear();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
    }

    fn press(app: &mut App, k: KeyCode) {
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(k);
    }

    fn release(app: &mut App, k: KeyCode) {
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(k);
    }

    /// Press and release one key over two frames.
    fn tap(app: &mut App, k: KeyCode) {
        press(app, k);
        step(app);
        release(app, k);
        step(app);
    }

    fn drag(app: &mut App, from: (f32, f32), to: (f32, f32)) {
        aim_at(app, from.0, from.1);
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
        step(app);
        aim_at(app, to.0, to.1);
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().release(MouseButton::Left);
        step(app);
    }

    fn click(app: &mut App, x: f32, z: f32) {
        drag(app, (x, z), (x, z));
    }

    fn sel(app: &App) -> Vec<EntityId> {
        app.world().resource::<Selection>().0.clone()
    }

    fn sorted(mut v: Vec<EntityId>) -> Vec<EntityId> {
        v.sort();
        v
    }

    #[test]
    fn click_selects_one_and_empty_space_clears() {
        let mut app = app();
        click(&mut app, 100.0, 100.0);
        assert_eq!(sel(&app), [A]);
        click(&mut app, 300.0, 100.0);
        assert_eq!(sel(&app), [B]);
        click(&mut app, 400.0, 400.0);
        assert!(sel(&app).is_empty());
    }

    #[test]
    fn shift_click_adds_and_removes() {
        let mut app = app();
        click(&mut app, 100.0, 100.0);
        press(&mut app, KeyCode::ShiftLeft);
        click(&mut app, 300.0, 100.0);
        assert_eq!(sel(&app), [A, B]);
        click(&mut app, 100.0, 100.0);
        assert_eq!(sel(&app), [B]);
    }

    #[test]
    fn box_selects_own_units_only() {
        let mut app = app();
        // Covers A, B and the enemy E, not C.
        drag(&mut app, (50.0, 50.0), (350.0, 350.0));
        assert_eq!(sel(&app), [A, B]);
        // A box with only an enemy in it selects nothing.
        drag(&mut app, (60.0, 260.0), (150.0, 350.0));
        assert!(sel(&app).is_empty());
    }

    #[test]
    fn enemy_is_inspected_alone() {
        let mut app = app();
        click(&mut app, 100.0, 100.0);
        click(&mut app, 100.0, 300.0);
        assert_eq!(sel(&app), [E]);
        assert!(own_selection(app.world().resource::<Selection>(), app.world().resource::<SimLink>()).is_empty());
        press(&mut app, KeyCode::ShiftLeft);
        click(&mut app, 300.0, 100.0);
        assert_eq!(sel(&app), [B]);
    }

    #[test]
    fn double_click_selects_visible_own_units_of_the_class() {
        let mut app = app();
        click(&mut app, 100.0, 100.0);
        click(&mut app, 100.0, 100.0);
        // A, B and D (class 1, on screen): not C (class 2), E (enemy), F (off screen).
        assert_eq!(sorted(sel(&app)), [A, B, D]);
    }

    #[test]
    fn select_type_key_takes_every_own_unit_of_the_class() {
        let mut app = app();
        click(&mut app, 100.0, 100.0);
        tap(&mut app, KeyCode::KeyT);
        assert_eq!(sorted(sel(&app)), [A, B, F, D]);
    }

    #[test]
    fn control_group_assign_recall_and_double_tap_focus() {
        let mut app = app();
        drag(&mut app, (50.0, 50.0), (350.0, 150.0));
        assert_eq!(sel(&app), [A, B]);
        press(&mut app, KeyCode::ControlLeft);
        tap(&mut app, KeyCode::Digit1);
        release(&mut app, KeyCode::ControlLeft);
        step(&mut app);
        click(&mut app, 500.0, 100.0);
        assert_eq!(sel(&app), [C]);
        tap(&mut app, KeyCode::Digit1);
        assert_eq!(sel(&app), [A, B]);
        tap(&mut app, KeyCode::Digit1);
        let focus: Vec<_> = app.world_mut().resource_mut::<Messages<CameraFocus>>().drain().collect();
        assert_eq!(focus.len(), 1);
        // The member nearest the mean (200, 100): A and B tie, the first wins.
        assert!((focus[0].0 - Vec3::new(100.0, 0.0, 100.0)).length() < 1.0, "{:?}", focus[0].0);
        // An empty group recalls nothing.
        tap(&mut app, KeyCode::Digit2);
        assert_eq!(sel(&app), [A, B]);
    }

    #[test]
    fn rebinding_moves_the_action_to_another_key() {
        let mut app = app();
        app.world_mut().resource_mut::<KeyBindings>().rebind(Action::SelectType, KeyCode::KeyQ);
        click(&mut app, 100.0, 100.0);
        tap(&mut app, KeyCode::KeyT);
        assert_eq!(sel(&app), [A]);
        tap(&mut app, KeyCode::KeyQ);
        assert_eq!(sorted(sel(&app)), [A, B, F, D]);
    }

    fn ctrl_digit(app: &mut App, digit: KeyCode) {
        press(app, KeyCode::ControlLeft);
        tap(app, digit);
        release(app, KeyCode::ControlLeft);
        step(app);
    }

    fn group(app: &App, n: usize) -> Vec<EntityId> {
        sorted(app.world().resource::<ControlGroups>().groups[n].clone())
    }

    #[test]
    fn shift_digit_adds_to_the_group_and_keeps_it_when_nothing_is_selected() {
        let mut app = app();
        click(&mut app, 100.0, 100.0);
        ctrl_digit(&mut app, KeyCode::Digit1);
        click(&mut app, 300.0, 100.0);
        press(&mut app, KeyCode::ShiftLeft);
        tap(&mut app, KeyCode::Digit1);
        release(&mut app, KeyCode::ShiftLeft);
        step(&mut app);
        assert_eq!(group(&app, 0), [A, B]);
        // Nothing selected: Shift+1 changes nothing.
        click(&mut app, 400.0, 400.0);
        press(&mut app, KeyCode::ShiftLeft);
        tap(&mut app, KeyCode::Digit1);
        release(&mut app, KeyCode::ShiftLeft);
        assert_eq!(group(&app, 0), [A, B]);
        // Shift+1 did not recall: the selection is still empty.
        assert!(sel(&app).is_empty());
    }

    #[test]
    fn ctrl_digit_replaces_and_empty_selection_empties() {
        let mut app = app();
        drag(&mut app, (50.0, 50.0), (350.0, 150.0));
        ctrl_digit(&mut app, KeyCode::Digit1);
        click(&mut app, 500.0, 100.0);
        ctrl_digit(&mut app, KeyCode::Digit1);
        assert_eq!(group(&app, 0), [C]);
        click(&mut app, 400.0, 400.0);
        ctrl_digit(&mut app, KeyCode::Digit1);
        assert!(group(&app, 0).is_empty());
    }

    #[test]
    fn a_unit_is_in_one_group_at_a_time() {
        let mut app = app();
        drag(&mut app, (50.0, 50.0), (350.0, 150.0));
        ctrl_digit(&mut app, KeyCode::Digit1);
        click(&mut app, 300.0, 100.0);
        ctrl_digit(&mut app, KeyCode::Digit2);
        assert_eq!(group(&app, 0), [A]);
        assert_eq!(group(&app, 1), [B]);
    }

    #[test]
    fn alt_digit_selects_and_centres_at_once() {
        let mut app = app();
        click(&mut app, 500.0, 100.0);
        ctrl_digit(&mut app, KeyCode::Digit3);
        click(&mut app, 100.0, 100.0);
        press(&mut app, KeyCode::AltLeft);
        tap(&mut app, KeyCode::Digit3);
        release(&mut app, KeyCode::AltLeft);
        assert_eq!(sel(&app), [C]);
        let focus: Vec<_> = app.world_mut().resource_mut::<Messages<CameraFocus>>().drain().collect();
        assert_eq!(focus.len(), 1);
        assert!((focus[0].0 - Vec3::new(500.0, 0.0, 100.0)).length() < 1.0);
    }

    #[test]
    fn selection_stops_at_the_cap() {
        assert_eq!(MaxSelection::new(5).get(), 17);
        assert_eq!(MaxSelection::new(500).get(), 120);
        assert_eq!(MaxSelection::default().get(), 40);
        let mut v: Vec<EntityId> = (0..30).map(EntityId).collect();
        cap(&mut v, MaxSelection::new(17));
        assert_eq!(v.len(), 17);
    }

    #[test]
    fn group_tap_window_is_750_ms() {
        assert_eq!(GROUP_TAP, 0.75);
    }
}
