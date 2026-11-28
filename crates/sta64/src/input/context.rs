//! Context orders (UI-14): what a right-click on a unit means for the
//! selection, decided by [`sta_sim::context::context_order`] (the
//! originals' group vote: the highest-priority order any selected unit
//! would take goes to the whole selection; no vote is a move, in
//! `orders.rs`). The same decision drives the cursor.

use bevy::prelude::*;
use bevy::window::{CursorIcon, PrimaryWindow, SystemCursorIcon};
use sta_sim::command::Order;

use crate::ui::hud::model::TargetMode;

use super::orders::Aim;
use super::select::{Targetable, own_selection, pickable, unit_under_where};
use crate::sim_link::{ContentRes, Selection, SimLink};

// The rule (the originals' group vote) lives in the sim crate, where the
// bench's `rclick` uses it too.
pub(super) use sta_sim::context::rally_builders;

/// The order a right-click on `target` gives `own`, or `None` for a plain move: the sim's group
/// vote, except that stationary builders take no order from a right-click. A rally point is set
/// only through the armed Rally command (the HUD's `TargetMode::Rally`), never by a plain
/// right-click (rule stated by a player, not measured; Armada I and the Fleet Operations family
/// follow it until shown otherwise). The sim's vote still answers `Order::Rally` for them, for the
/// bench's `rclick`; the client drops it here so the cursor and the click share one decision.
pub fn context_order(db: Option<&sta_content::ContentDb>, view: &sta_sim::WorldView, me: sta_sim::PlayerId, own: &[sta_sim::EntityId], target: sta_sim::EntityId) -> Option<Order> {
    sta_sim::context::context_order(db, view, me, own, target).filter(|o| !matches!(o, Order::Rally { .. }))
}

/// Everything the cursor decision reads, reduced to what it needs. Built by [`cursor`] from the
/// same inputs the right-click (`orders::order`) reads, so the cursor and the order are one decision.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Situation<'a> {
    /// A cutscene runs: the arrow only (read from the executable, static).
    pub cutscene: bool,
    /// The pointer is over a HUD panel: the default arrow (measured).
    pub over_hud: bool,
    /// A HUD pick is armed: the mode and whether the click would give an order (the HUD's own rule).
    pub armed: Option<(TargetMode, bool)>,
    /// The selection holds own units (a foreign unit inspected alone is not a selection to order with).
    pub own_selected: bool,
    /// The order the right-click gives on the unit under the pointer, from the group vote.
    pub hover: Option<&'a Order>,
    /// The right-click on open space or on a unit without an order would move someone: some selected
    /// unit is not a builder that is busy building (the sim's `BUILDING` flag), and the pointer's ray
    /// meets the map plane.
    pub moves: bool,
    /// The click on open ground would be a warp move (`orders::world_warp`: Alt held, on Armada II
    /// content, and warp on offer: `sim_link::warp_allowed`; with the queue modifier it is a queued warp move).
    pub warp: bool,
    /// The pointer is over the minimap's square (it is HUD, but its right-click is an order).
    pub minimap: Option<MinimapPointer>,
}

/// The pointer over the minimap, reduced to what the cursor needs (what `orders::radar_order` reads).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MinimapPointer {
    /// The map point under it is inside the map's rectangle, not in a strip.
    pub inside_map: bool,
    /// The click is a plain move, not a warp move (`orders::minimap_plain`: Alt or Armada I content).
    pub plain: bool,
    /// Some selected own unit would be moved (not a builder busy building).
    pub movers: bool,
}

/// The cursor sprite for what a right-click would do (names are the install's own `cursor.spr` keys).
///
/// Rows (grades): no own selection gives `c_select` (measured); a mobile own selection over open
/// space or any unit without a vote gives `c_move` (measured); the vote's own order gives its cursor
/// (attack measured, the others read from the executable, static: mine, repair, wormhole); over a HUD
/// panel and during a cutscene the arrow. INTERIM, not measured: a selection whose right-click gives
/// nothing (only builders busy building, or a ray that misses the map plane) shows `c_select`.
/// Stationary builders (a yard, a station) give no order on a right-click and so show `c_select`
/// too; their rally point is set only by the armed Rally command, which shows `c_rally` (rule
/// stated by a player, 2026-11-22, not measured; Armada I and the Fleet Operations family follow it
/// until shown otherwise). Armed picks use the HUD's table; where STA64 gives no order the refusal cursor shows
/// (the original gives attack, patrol and guard on the ground or on own ships; STA64 has no such
/// order yet, so its present behaviour stays, interim).
pub(crate) fn decide(s: &Situation) -> &'static str {
    use crate::ui::cursor::DEFAULT;
    if s.cutscene {
        return DEFAULT;
    }
    // Over the minimap: what its right-click gives (`orders::radar_order`): with an own selection
    // and a point inside the map, a warp move (cursor `c_warp`, measured)
    // or, with Alt, a plain move (`c_move`, read from the executable, static); otherwise the
    // panel's arrow (no selection: measured; a strip or armed pick: the click does nothing).
    if let Some(m) = s.minimap {
        return match (s.own_selected && m.inside_map && s.armed.is_none(), m.movers, m.plain) {
            (true, true, false) => "c_warp",
            (true, true, true) => "c_move",
            _ => DEFAULT,
        };
    }
    if s.over_hud {
        return DEFAULT;
    }
    if let Some((mode, valid)) = s.armed {
        return crate::ui::hud::armed_cursor_name(mode, valid);
    }
    if !s.own_selected {
        return "c_select";
    }
    match s.hover {
        Some(Order::Attack { .. }) => "c_attack",
        Some(Order::Mine { .. }) => "c_mining",
        Some(Order::Repair { .. }) => "c_repair",
        // A wormhole is the one target `context_order` answers with a move.
        Some(Order::Move { .. }) => "c_wormin",
        // Alt over a place a selected own craft would be moved to: the warp cursor (read from the
        // executable, static). INTERIM, narrowest reading: only where the plain cursor would be
        // `c_move`, so any mover counts (as on the minimap), and a wormhole or a target with an
        // order of its own keeps its cursor.
        _ if s.moves && s.warp => "c_warp",
        _ if s.moves => "c_move",
        _ => "c_select",
    }
}

/// Does the ray meet the horizontal plane at height `y` in front of its origin (as `orders::order`
/// needs to place a move)?
fn plane_hit(ray: Ray3d, y: f32) -> bool {
    let dir = *ray.direction;
    dir.y.abs() >= 1e-4 && (y - ray.origin.y) / dir.y > 0.0
}

/// The cursor shows the order a right-click would give: the crosshair for an attack, the move
/// target for a move, and so on (see [`decide`]).
#[allow(clippy::too_many_arguments)] // Bevy injects these system parameters.
pub fn cursor(
    aim: Res<Aim>,
    units: Targetable,
    selection: Res<Selection>,
    link: Res<SimLink>,
    content: Option<Res<ContentRes>>,
    local: Option<Res<crate::render::units::LocalPlayer>>,
    mut windows: Query<&mut CursorIcon, With<PrimaryWindow>>,
    mut commands: Commands,
    window: Query<Entity, With<PrimaryWindow>>,
    hud: Option<Res<crate::ui::hud::HudPointer>>,
    mut wanted: ResMut<crate::ui::cursor::WantedCursor>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    radar: Query<&bevy::ui::RelativeCursorPosition, With<crate::ui::radar::RadarSquare>>,
    radar_state: Option<Res<crate::ui::radar::RadarState>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
) {
    // Over a cutscene no order is on offer (selection and orders are gated the same way), so the
    // cursor stays the arrow: the selection is kept, inert, and the cursor reads it again afterwards.
    let cutscene = crate::ui::presentation::cutscene_running(cine);
    let own = if cutscene { Vec::new() } else { own_selection(&selection, &link) };
    let icon = aim.0.filter(|_| !own.is_empty()).and_then(|a| unit_under_where(a.cursor, a.ray, &units, |t| pickable(link.curr(), local.as_deref().and_then(|l| l.0), t))).and_then(|t| {
        context_order(content.as_deref().map(|c| &*c.0), link.curr(), link.player(), &own, t)
    });
    // Stationary builders take no order from a right-click (a rally is set only by the armed Rally
    // command): nothing is moved, so the cursor is the one of a click that gives nothing.
    let passive = rally_builders(content.as_deref().map(|c| &*c.0), link.curr(), link.player(), &own);
    // The move `orders::order` would give: units that are not busy building, a point on their mean-height plane.
    let heights: Vec<f32> = own.iter().filter(|_| !passive).filter_map(|&id| link.curr().unit(id)).filter(|u| !u.flags.contains(sta_sim::view::UnitFlags::BUILDING)).map(|u| u.pos.to_f32()[1]).collect();
    let moves = !heights.is_empty() && aim.0.is_some_and(|a| plane_hit(a.ray, heights.iter().sum::<f32>() / heights.len() as f32));
    // The minimap's right-click needs no ray, only a point inside the map.
    let minimap = radar.iter().find(|r| r.cursor_over).zip(radar_state.as_deref().and_then(|s| s.map.map(|m| (s, m)))).and_then(|(r, (s, m))| {
        let at = crate::ui::radar::click_at(s, m, r.normalized?, bevy::picking::pointer::PointerButton::Secondary);
        Some(MinimapPointer {
            inside_map: at.inside_map,
            plain: keys.as_deref().is_some_and(|k| super::orders::minimap_plain(k, content.as_deref().map(|c| &*c.0))) || !crate::sim_link::warp_allowed(link.curr(), content.as_deref().map(|c| &*c.0)),
            movers: !heights.is_empty(),
        })
    });
    // The armed pick (the HUD's system) writes over this when it is on.
    let over_hud = hud.is_some_and(|h| h.0);
    let warp = keys.as_deref().is_some_and(|k| super::orders::world_warp(k, content.as_deref().map(|c| &*c.0))) && crate::sim_link::warp_allowed(link.curr(), content.as_deref().map(|c| &*c.0));
    wanted.0 = decide(&Situation { cutscene, over_hud, armed: None, own_selected: !own.is_empty(), hover: icon.as_ref(), moves, warp, minimap });
    let want = match icon {
        Some(Order::Attack { .. }) => SystemCursorIcon::Crosshair,
        Some(Order::Mine { .. }) => SystemCursorIcon::Pointer,
        _ => SystemCursorIcon::Default,
    };
    let want = CursorIcon::from(want);
    match (windows.single_mut(), window.single()) {
        (Ok(mut cur), _) => {
            if *cur != want {
                *cur = want;
            }
        }
        (_, Ok(w)) => {
            commands.entity(w).insert(want);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::ecs::system::RunSystemOnce;
    use sta_content::{ContentDb, DefRef, UnitDef, fake};
    use sta_core::{Fixed, TeamId};
    use sta_sim::command::Target;
    use sta_sim::view::{Relation, UnitFlags};
    use sta_sim::{EntityId, WorldView};
    use sta_sim::command::{CommandKind, LocalRouter};
    use sta_sim::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use sta_sim::{PlayerId, Sim, TickOutput, Vec3 as SimVec3, WorldSource};

    const A2: sta_content::Profile = sta_content::Profile::Sta2Classic;

    use super::super::keys::KeyBindings;
    use super::super::orders::{OrderTargets, order};
    use super::super::select::AimState;
    use super::*;
    use crate::render::units::SelectRadius;
    use crate::sim_link::{CommandQueue, OrderIssued, OrderKind, SimEntity};

    struct ViewSource(Arc<WorldView>);

    impl WorldSource for ViewSource {
        fn step(&mut self, _: &[sta_sim::command::Command]) -> TickOutput {
            TickOutput { view: self.0.clone(), events: Vec::new() }
        }

        fn view(&self) -> Arc<WorldView> {
            self.0.clone()
        }
    }

    /// Seat 0 (us) has a miner and a cruiser; seat 1 a cruiser; a moon owned
    /// by seat 0 (the wrongly-owned case) and one owned by nobody. Returns
    /// ids `[miner, cruiser, enemy, moon_owned, moon_free, yard, rig, second_yard, ally, station]`.
    fn app(select: usize) -> (App, [EntityId; 10]) {
        app_as(select, fake::db().profile())
    }

    fn app_as(select: usize, profile: sta_content::Profile) -> (App, [EntityId; 10]) {
        let fake = fake::db();
        let mut builder = ContentDb::builder().profile(profile).features(fake.features().clone());
        for unit in fake.units() {
            builder = builder.unit(unit.clone());
        }
        for weapon in fake.weapons() {
            builder = builder.weapon(weapon.clone());
        }
        let mut rig = UnitDef::new("fx_rig", "constructionrig");
        rig.movement = fake.unit(fake.key("fx_cruiser").unwrap()).unwrap().movement;
        rig.build.list = vec![DefRef::new("fx_station")];
        rig.hull = Fixed::from_int(100);
        // A station that builds nothing: no rally, and not busy building.
        let mut turret = UnitDef::new("fx_turret", "starbase");
        turret.hull = Fixed::from_int(100);
        let db = Arc::new(builder.unit(rig).unit(turret).build().unwrap());
        let seat = |id, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
        let setup = GameSetup {
            seed: 1,
            map: Arc::new(fake::two_bases()),
            content: db.clone(),
            players: vec![
                seat(0, Control::Human),
                seat(1, Control::Idle),
                PlayerSetup { id: PlayerId(2), team: TeamId(0), race: "fx".into(), control: Control::Idle },
            ],
            options: GameOptions::default(),
        };
        let mut sim = Sim::from_setup(&setup);
        let h = (Fixed::ZERO, Fixed::ONE);
        let at = |x: f32| SimVec3::from_f32([x, 0.0, 0.0]);
        let mut spawn = |odf: &str, owner: u8, x: f32| sim.spawn_def(db.key(odf).unwrap(), PlayerId(owner), at(x), h).unwrap();
        let ids = [spawn("fx_miner", 0, 0.0), spawn("fx_cruiser", 0, 100.0), spawn("fx_cruiser", 1, -3000.0), spawn("fx_moon", 0, 3000.0), spawn("fx_moon", 255, 6000.0), spawn("fx_yard", 0, 9000.0), spawn("fx_rig", 0, 12000.0), spawn("fx_yard", 0, 15000.0), spawn("fx_cruiser", 2, -6000.0), spawn("fx_turret", 0, 18000.0)];
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<OrderIssued>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<CommandQueue>()
            .init_resource::<OrderTargets>()
            .init_resource::<KeyBindings>()
            .init_resource::<Aim>()
            .insert_resource(ContentRes(db))
            .insert_resource(Selection(vec![ids[select]]))
            .insert_resource(SimLink::new(Box::new(sim), Box::new(LocalRouter::new()), PlayerId(0)))
            .add_systems(Update, order);
        for id in ids {
            let x = app.world().resource::<SimLink>().curr().unit(id).map_or(0.0, |u| u.pos.to_f32()[0]);
            app.world_mut().spawn((SimEntity(id), SelectRadius(50.0), GlobalTransform::from(Transform::from_xyz(x, 0.0, 0.0))));
        }
        (app, ids)
    }

    fn right_click_at(app: &mut App, x: f32) -> Vec<CommandKind> {
        right_click_at_xz(app, x, 0.0)
    }

    fn right_click_at_xz(app: &mut App, x: f32, z: f32) -> Vec<CommandKind> {
        let ray = Ray3d::new(Vec3::new(x, 1000.0, z), Dir3::NEG_Y);
        app.world_mut().resource_mut::<Aim>().0 = Some(AimState { cursor: Vec2::ZERO, ray, clip_from_world: Mat4::IDENTITY, size: Vec2::new(800.0, 600.0) });
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Right);
        app.update();
        std::mem::take(&mut app.world_mut().resource_mut::<CommandQueue>().0)
    }

    fn situation<'a>(hover: Option<&'a Order>) -> Situation<'a> {
        Situation { own_selected: true, moves: true, hover, ..Default::default() }
    }

    /// Over the minimap the cursor shows what its right-click gives: the warp cursor with a movable
    /// own selection inside the map, the move cursor with Alt; the arrow without a selection, in a
    /// strip, with nothing to move (stationary builders included) and in a cutscene.
    #[test]
    fn decide_over_the_minimap_follows_the_warp_move_the_click_gives() {
        let pointer = MinimapPointer { inside_map: true, plain: false, movers: true };
        let on = |p: MinimapPointer| Situation { own_selected: true, over_hud: true, minimap: Some(p), ..Default::default() };
        assert_eq!(decide(&on(pointer)), "c_warp");
        assert_eq!(decide(&on(MinimapPointer { plain: true, ..pointer })), "c_move");
        let arrow = crate::ui::cursor::DEFAULT;
        assert_eq!(decide(&on(MinimapPointer { inside_map: false, ..pointer })), arrow, "a strip: the click does nothing");
        assert_eq!(decide(&on(MinimapPointer { movers: false, ..pointer })), arrow);
        assert_eq!(decide(&Situation { own_selected: false, ..on(pointer) }), arrow, "no selection: the panel's arrow");
        assert_eq!(decide(&Situation { cutscene: true, ..on(pointer) }), arrow);
        assert_eq!(decide(&Situation { armed: Some((TargetMode::Attack, true)), ..on(pointer) }), arrow, "armed picks are not aimed from the minimap");
    }

    #[test]
    fn decide_warp_cursor_only_where_the_plain_one_is_move() {
        let warp = |hover| Situation { warp: true, ..situation(hover) };
        assert_eq!(decide(&warp(None)), "c_warp");
        assert_eq!(decide(&situation(None)), "c_move");
        assert_eq!(decide(&Situation { moves: false, ..warp(None) }), "c_select");
        assert_eq!(decide(&Situation { own_selected: false, ..warp(None) }), "c_select");
        let wormhole = Order::Move { units: vec![EntityId(1)], to: SimVec3::from_f32([0.0; 3]), queue: false, mode: None };
        assert_eq!(decide(&warp(Some(&wormhole))), "c_wormin");
        assert_eq!(decide(&Situation { over_hud: true, ..warp(None) }), crate::ui::cursor::DEFAULT);
        assert_eq!(decide(&Situation { armed: Some((TargetMode::Attack, true)), ..warp(None) }), "c_attack");
    }

    #[test]
    fn decide_gives_the_cursor_of_each_order_and_move_by_default() {
        let u = vec![EntityId(1)];
        let attack = Order::Attack { units: u.clone(), target: EntityId(2) };
        let mine = Order::Mine { units: u.clone(), node: EntityId(2) };
        let repair = Order::Repair { units: u.clone(), yard: Some(EntityId(2)) };
        let wormhole = Order::Move { units: u, to: SimVec3::from_f32([0.0; 3]), queue: false, mode: None };
        assert_eq!(decide(&situation(Some(&attack))), "c_attack");
        assert_eq!(decide(&situation(Some(&mine))), "c_mining");
        assert_eq!(decide(&situation(Some(&repair))), "c_repair");
        assert_eq!(decide(&situation(Some(&wormhole))), "c_wormin");
        // Open space, or a unit without a vote (an own ship, a moon for warships): the move cursor.
        assert_eq!(decide(&situation(None)), "c_move");
    }

    #[test]
    fn decide_without_own_selection_is_select_and_without_a_move_it_is_select() {
        let none = Situation { moves: true, ..Default::default() };
        assert_eq!(decide(&none), "c_select");
        // Own units selected but the click would do nothing (only a busy builder).
        assert_eq!(decide(&Situation { own_selected: true, ..Default::default() }), "c_select");
    }

    #[test]
    fn decide_arrow_over_hud_and_cutscene_and_armed_picks_win() {
        let attack = Order::Attack { units: vec![EntityId(1)], target: EntityId(2) };
        let over = Situation { over_hud: true, ..situation(Some(&attack)) };
        assert_eq!(decide(&over), crate::ui::cursor::DEFAULT);
        let cine = Situation { cutscene: true, ..situation(Some(&attack)) };
        assert_eq!(decide(&cine), crate::ui::cursor::DEFAULT);
        let armed = |m, v| decide(&Situation { armed: Some((m, v)), ..situation(None) });
        assert_eq!(armed(TargetMode::Move, true), "c_move");
        assert_eq!(armed(TargetMode::Move, false), "c_nomove");
        assert_eq!(armed(TargetMode::Attack, true), "c_attack");
        assert_eq!(armed(TargetMode::Attack, false), "c_noattack");
        assert_eq!(armed(TargetMode::Mine, true), "c_mining");
        assert_eq!(armed(TargetMode::Rally, false), "c_nomove");
        // The HUD panel still wins over an armed pick.
        assert_eq!(decide(&Situation { over_hud: true, armed: Some((TargetMode::Attack, true)), ..situation(None) }), crate::ui::cursor::DEFAULT);
    }

    #[test]
    fn plane_hit_needs_a_ray_that_reaches_the_plane() {
        assert!(plane_hit(Ray3d::new(Vec3::new(0.0, 1000.0, 0.0), Dir3::NEG_Y), 0.0));
        assert!(!plane_hit(Ray3d::new(Vec3::new(0.0, 1000.0, 0.0), Dir3::Y), 0.0));
        assert!(!plane_hit(Ray3d::new(Vec3::new(0.0, 1000.0, 0.0), Dir3::X), 0.0));
    }

    /// The kind of the first command of a click, as a cursor key it must agree with.
    fn kind_cursor(cmds: &[CommandKind]) -> Option<&'static str> {
        match cmds.first()? {
            CommandKind::Order(Order::Attack { .. }) => Some("c_attack"),
            CommandKind::Order(Order::Mine { .. }) => Some("c_mining"),
            CommandKind::Order(Order::Move { .. }) => Some("c_move"),
            CommandKind::Order(Order::MoveWarp { .. }) => Some("c_warp"),
            _ => None,
        }
    }

    /// Cursor and order are one decision: for each situation, the cursor the system wants is the
    /// one of the order a right-click gives in the same frame, and the click gives one.
    #[test]
    fn the_cursor_matches_the_order_a_right_click_gives() {
        let cases: [(usize, f32, &str); 8] = [
            (1, 20000.0, "c_move"),   // a cruiser over open space
            (1, -6000.0, "c_move"),   // over an ally's ship
            (1, 6000.0, "c_move"),    // a warship over a free moon
            (1, -3000.0, "c_attack"), // over an enemy
            (0, 3000.0, "c_mining"),  // a miner over a moon
            (5, 10000.0, "c_select"), // a stationary yard: the click gives nothing (a rally is armed)
            (6, 13000.0, "c_move"),   // a mobile rig
            (9, 19000.0, "c_move"),   // a station that builds nothing: the click moves it today
        ];
        for (select, x, cursor_name) in cases {
            let (mut app, _) = app(select);
            app.init_resource::<crate::ui::cursor::WantedCursor>();
            let cmds = right_click_at(&mut app, x);
            app.world_mut().run_system_once(cursor).unwrap();
            assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, cursor_name, "select {select} at {x}: {cmds:?}");
            assert_eq!(kind_cursor(&cmds).unwrap_or("c_select"), cursor_name, "select {select} at {x}: the click must give the order of the cursor: {cmds:?}");
            assert_eq!(cmds.is_empty(), cursor_name == "c_select", "select {select} at {x}: {cmds:?}");
        }
    }

    /// Alt held in the world: the warp cursor exactly where the click gives the warp move (open
    /// space, an own craft, a free moon for a warship), the cursors of the targets with an order of
    /// their own unchanged, stationary builders still without an order, and no warp cursor without Alt
    /// or without a mover. With the queue modifier too it is the warp cursor and a queued warp move. The click goes through the minimap's order path.
    #[test]
    fn alt_gives_the_warp_cursor_and_the_warp_move_where_the_plain_cursor_is_move() {
        let cases: [(usize, f32, &str); 8] = [
            (1, 20000.0, "c_warp"),   // open space
            (1, -6000.0, "c_warp"),   // over an ally's ship
            (1, 6000.0, "c_warp"),    // a warship over a free moon
            (1, -3000.0, "c_attack"), // over an enemy: its own order
            (0, 3000.0, "c_mining"),  // a miner over a moon
            (5, 10000.0, "c_select"), // a stationary yard: no order, no warp
            (6, 13000.0, "c_warp"),   // a mobile rig
            (9, 19000.0, "c_warp"),   // a station that builds nothing (as the plain click)
        ];
        for (select, x, cursor_name) in cases {
            let (mut app, _) = app_as(select, A2);
            app.init_resource::<crate::ui::cursor::WantedCursor>();
            app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::AltLeft);
            let cmds = right_click_at(&mut app, x);
            app.world_mut().run_system_once(cursor).unwrap();
            assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, cursor_name, "select {select} at {x}: {cmds:?}");
            assert_eq!(kind_cursor(&cmds).unwrap_or("c_select"), cursor_name, "select {select} at {x}: {cmds:?}");
            assert_eq!(cmds.is_empty(), cursor_name == "c_select", "select {select} at {x}: {cmds:?}");
        }
        // The queue modifier keeps the warp cursor and gives a queued warp move (STA64's own rule, #1173).
        let (mut app2, _) = app_as(1, A2);
        app2.init_resource::<crate::ui::cursor::WantedCursor>();
        for k in [KeyCode::AltLeft, KeyCode::ShiftLeft] {
            app2.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(k);
        }
        let cmds = right_click_at(&mut app2, 20000.0);
        app2.world_mut().run_system_once(cursor).unwrap();
        assert_eq!(app2.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_warp");
        assert_eq!(kind_cursor(&cmds), Some("c_warp"));
        assert!(matches!(&cmds[..], [CommandKind::Order(Order::MoveWarp { queue: true, .. })]), "{cmds:?}");
        // Warp off: no warp cursor and a plain move, even with Alt held and with the queue modifier too. Each of the
        // three terms: the content's own switch, the mission switch the view carries and the match option.
        struct Frozen(Arc<WorldView>);
        impl WorldSource for Frozen {
            fn step(&mut self, _: &[sta_sim::command::Command]) -> TickOutput {
                TickOutput { view: self.0.clone(), events: vec![] }
            }
            fn view(&self) -> Arc<WorldView> {
                self.0.clone()
            }
        }
        for (term, queue) in [("content", false), ("mission switch", false), ("match option", false), ("content", true), ("match option", true)] {
            let (mut app4, _) = app_as(1, A2);
            app4.init_resource::<crate::ui::cursor::WantedCursor>();
            match term {
                "content" => {
                    let db = (*app4.world().resource::<ContentRes>().0).clone();
                    app4.insert_resource(ContentRes(Arc::new(db.clone().with_rules(db.rules().clone().with_warp(Some(false))))));
                }
                other => {
                    let mut view = (**app4.world().resource::<SimLink>().curr()).clone();
                    if other == "mission switch" {
                        view.attrs.mission_switches.allow_warp = false;
                    } else {
                        view.attrs.match_warp_off = true;
                    }
                    app4.insert_resource(SimLink::new(Box::new(Frozen(Arc::new(view))), Box::new(LocalRouter::new()), PlayerId(0)));
                }
            }
            app4.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::AltLeft);
            if queue {
                app4.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::ShiftLeft);
            }
            let cmds = right_click_at(&mut app4, 20000.0);
            app4.world_mut().run_system_once(cursor).unwrap();
            assert_eq!(app4.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_move", "{term} queue {queue}: {cmds:?}");
            assert_eq!(kind_cursor(&cmds), Some("c_move"), "{term} queue {queue}: {cmds:?}");
            if queue {
                assert!(matches!(&cmds[..], [CommandKind::Order(Order::Move { queue: true, .. })]), "queue + warp off = a queued PLAIN move: {cmds:?}");
            }
        }
        // A pointer whose ray misses the map plane: nothing to warp to.
        let (mut app3, _) = app_as(1, A2);
        app3.init_resource::<crate::ui::cursor::WantedCursor>();
        app3.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::AltLeft);
        let ray = Ray3d::new(Vec3::new(0.0, 1000.0, 0.0), Dir3::X);
        app3.world_mut().resource_mut::<Aim>().0 = Some(AimState { cursor: Vec2::ZERO, ray, clip_from_world: Mat4::IDENTITY, size: Vec2::new(800.0, 600.0) });
        app3.world_mut().run_system_once(cursor).unwrap();
        assert_eq!(app3.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_select");
    }

    /// The rule was read from Armada II only: Fleet Operations, Roots, Armada I and STA64's own
    /// content keep the plain move and the move cursor on Alt (INTERIM).
    #[test]
    fn alt_keeps_the_plain_move_and_cursor_on_other_profiles() {
        use sta_content::Profile::*;
        for profile in [Fo3, Fo4, Sta1Classic, Native] {
            let (mut app, _) = app_as(1, profile);
            app.init_resource::<crate::ui::cursor::WantedCursor>();
            app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::AltLeft);
            let cmds = right_click_at(&mut app, 20000.0);
            app.world_mut().run_system_once(cursor).unwrap();
            assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_move", "{profile:?}");
            assert_eq!(kind_cursor(&cmds), Some("c_move"), "{profile:?}: {cmds:?}");
        }
    }

    /// Where the click gives nothing the cursor is the select cursor: no own selection; a builder busy
    /// building (the sim's `BUILDING` flag); a ray that never meets the map plane.
    #[test]
    fn the_select_cursor_shows_where_the_click_gives_nothing() {
        let wanted = |app: &App| app.world().resource::<crate::ui::cursor::WantedCursor>().0;
        // No own selection.
        let (mut app, _) = self::app(1);
        app.init_resource::<crate::ui::cursor::WantedCursor>();
        app.world_mut().insert_resource(Selection(Vec::new()));
        assert!(right_click_at(&mut app, 20000.0).is_empty());
        app.world_mut().run_system_once(cursor).unwrap();
        assert_eq!(wanted(&app), "c_select");
        // An own rig busy building.
        let (mut app, ids) = self::app(6);
        app.init_resource::<crate::ui::cursor::WantedCursor>();
        let mut view = (**app.world().resource::<SimLink>().curr()).clone();
        let rig = view.units.iter_mut().find(|u| u.id == ids[6]).unwrap();
        rig.flags = UnitFlags(rig.flags.0 | UnitFlags::BUILDING.0);
        app.insert_resource(SimLink::new(Box::new(ViewSource(Arc::new(view))), Box::new(LocalRouter::new()), PlayerId(0)));
        assert!(right_click_at(&mut app, 13000.0).is_empty());
        app.world_mut().run_system_once(cursor).unwrap();
        assert_eq!(wanted(&app), "c_select");
        // A ray above the horizon: no ground point, so no order, for a ship and for a yard.
        for select in [1, 5] {
            let (mut app, _) = self::app(select);
            app.init_resource::<crate::ui::cursor::WantedCursor>();
            let ray = Ray3d::new(Vec3::new(0.0, 1000.0, 0.0), Dir3::Y);
            app.world_mut().resource_mut::<Aim>().0 = Some(AimState { cursor: Vec2::ZERO, ray, clip_from_world: Mat4::IDENTITY, size: Vec2::new(800.0, 600.0) });
            app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Right);
            app.update();
            assert!(app.world().resource::<CommandQueue>().0.is_empty(), "select {select}");
            app.world_mut().run_system_once(cursor).unwrap();
            assert_eq!(wanted(&app), "c_select", "select {select}");
        }
    }

    #[test]
    fn order_cursor_reaches_seen_enemy_behind_unseen_moon_but_keeps_seen_moon_gate() {
        let (mut app, ids) = app(1);
        app.insert_resource(crate::render::units::LocalPlayer(Some(PlayerId(0))));
        app.init_resource::<crate::ui::cursor::WantedCursor>();
        let mut view = (**app.world().resource::<SimLink>().curr()).clone();
        view.visibility = vec![(PlayerId(0), view.units.iter().map(|u| u.id).filter(|&id| id != ids[4]).collect())];
        app.insert_resource(SimLink::new(Box::new(ViewSource(Arc::new(view))), Box::new(LocalRouter::new()), PlayerId(0)));
        let front = app.world_mut().query::<(Entity, &SimEntity)>().iter(app.world()).find(|(_, id)| id.0 == ids[4]).unwrap().0;
        app.world_mut().entity_mut(front).insert(GlobalTransform::from(Transform::from_xyz(-3000.0, 100.0, 0.0)));
        app.world_mut().resource_mut::<Aim>().0 = Some(AimState {
            cursor: Vec2::ZERO,
            ray: Ray3d::new(Vec3::new(-3000.0, 1000.0, 0.0), Dir3::NEG_Y),
            clip_from_world: Mat4::IDENTITY,
            size: Vec2::new(800.0, 600.0),
        });
        app.world_mut().run_system_once(cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_attack");

        let mut view = (**app.world().resource::<SimLink>().curr()).clone();
        view.visibility[0].1.push(ids[4]);
        view.visibility[0].1.sort();
        app.insert_resource(SimLink::new(Box::new(ViewSource(Arc::new(view))), Box::new(LocalRouter::new()), PlayerId(0)));
        app.world_mut().run_system_once(cursor).unwrap();
        // A seen moon is the target: a warship has no vote on it, so the click is a move.
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_move");
    }

    /// The attack crosshair is not shown over a cutscene; it returns after it.
    #[test]
    fn the_order_cursor_is_not_shown_during_a_cutscene() {
        let (mut app, ids) = app(1);
        app.insert_resource(crate::render::units::LocalPlayer(Some(PlayerId(0))));
        app.init_resource::<crate::ui::cursor::WantedCursor>();
        let mut view = (**app.world().resource::<SimLink>().curr()).clone();
        view.visibility = vec![(PlayerId(0), view.units.iter().map(|u| u.id).filter(|&id| id != ids[4]).collect())];
        app.insert_resource(SimLink::new(Box::new(ViewSource(Arc::new(view))), Box::new(LocalRouter::new()), PlayerId(0)));
        let front = app.world_mut().query::<(Entity, &SimEntity)>().iter(app.world()).find(|(_, id)| id.0 == ids[4]).unwrap().0;
        app.world_mut().entity_mut(front).insert(GlobalTransform::from(Transform::from_xyz(-3000.0, 100.0, 0.0)));
        app.world_mut().resource_mut::<Aim>().0 = Some(AimState {
            cursor: Vec2::ZERO,
            ray: Ray3d::new(Vec3::new(-3000.0, 1000.0, 0.0), Dir3::NEG_Y),
            clip_from_world: Mat4::IDENTITY,
            size: Vec2::new(800.0, 600.0),
        });
        let wanted = |app: &App| app.world().resource::<crate::ui::cursor::WantedCursor>().0;
        app.world_mut().run_system_once(cursor).unwrap();
        assert_eq!(wanted(&app), "c_attack", "the rig shows it without a cutscene");
        app.insert_resource(crate::ui::presentation::CineractiveState { on: true, skippable: false });
        app.world_mut().run_system_once(cursor).unwrap();
        assert_eq!(wanted(&app), crate::ui::cursor::DEFAULT);
        app.insert_resource(crate::ui::presentation::CineractiveState { on: false, skippable: false });
        app.world_mut().run_system_once(cursor).unwrap();
        assert_eq!(wanted(&app), "c_attack");
    }

    #[test]
    fn freighter_on_a_moon_mines_whoever_owns_it() {
        for (x, moon) in [(3000.0, 3), (6000.0, 4)] {
            let (mut app, ids) = app(0);
            let cmds = right_click_at(&mut app, x);
            assert_eq!(cmds, [CommandKind::Order(Order::Mine { units: vec![ids[0]], node: ids[moon] })]);
            let msgs: Vec<_> = app.world_mut().resource_mut::<Messages<OrderIssued>>().drain().collect();
            assert_eq!(msgs[0].kind, OrderKind::Mine);
        }
    }

    #[test]
    fn warship_on_a_moon_moves_instead() {
        let (mut app, _) = app(1);
        let cmds = right_click_at(&mut app, 6000.0);
        assert!(matches!(cmds[..], [CommandKind::Order(Order::Move { .. })]), "{cmds:?}");
    }

    #[test]
    fn ship_on_an_enemy_attacks() {
        let (mut app, ids) = app(1);
        let cmds = right_click_at(&mut app, -3000.0);
        assert_eq!(cmds, [CommandKind::Order(Order::Attack { units: vec![ids[1]], target: ids[2] })]);
    }

    #[test]
    fn ship_on_an_ally_moves_and_yard_gives_no_order_and_the_normal_cursor() {
        let (mut ship_app, _) = app(1);
        assert!(matches!(right_click_at(&mut ship_app, -6000.0)[..], [CommandKind::Order(Order::Move { .. })]));

        // Stationary builders over a friendly unit: no order, and the cursor agrees.
        let (mut yard_app, _) = app(5);
        yard_app.init_resource::<crate::ui::cursor::WantedCursor>();
        assert!(right_click_at(&mut yard_app, -6000.0).is_empty());
        yard_app.world_mut().run_system_once(cursor).unwrap();
        assert_eq!(yard_app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_select");
    }

    #[test]
    fn neutral_relation_does_not_issue_attack() {
        let (mut app, _) = app(1);
        let link = app.world().resource::<SimLink>();
        let mut view = link.curr().as_ref().clone();
        let player = view.players.iter_mut().find(|p| p.id == PlayerId(0)).unwrap();
        player.relations.iter_mut().find(|(id, _)| *id == PlayerId(1)).unwrap().1 = Relation::Neutral;
        app.world_mut().insert_resource(SimLink::new(Box::new(ViewSource(Arc::new(view))), Box::new(LocalRouter::new()), PlayerId(0)));
        assert!(matches!(right_click_at(&mut app, -3000.0)[..], [CommandKind::Order(Order::Move { .. })]));
    }

    /// Stationary builders and nothing armed: a right-click gives no order and no acknowledgement,
    /// over open space and over a friendly unit, and the cursor is the one of a click that does nothing.
    #[test]
    fn yard_right_clicks_give_no_order_and_the_normal_cursor() {
        for x in [10000.0, 100.0] {
            let (mut app, _) = app(5);
            app.init_resource::<crate::ui::cursor::WantedCursor>();
            assert!(right_click_at(&mut app, x).is_empty(), "x {x}");
            assert!(app.world_mut().resource_mut::<Messages<OrderIssued>>().drain().next().is_none());
            app.world_mut().run_system_once(cursor).unwrap();
            assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_select", "x {x}");
        }
    }

    #[test]
    fn mixed_group_on_a_moon_sends_mine_to_the_whole_selection() {
        let (mut app, ids) = app(1);
        app.world_mut().insert_resource(Selection(vec![ids[0], ids[1]]));
        let cmds = right_click_at(&mut app, 6000.0);
        assert!(
            matches!(&cmds[..], [CommandKind::Order(Order::Mine { units, node })] if units.len() == 2 && *node == ids[4]),
            "{cmds:?}"
        );
    }

    #[test]
    fn mobile_builder_and_mixed_group_keep_move() {
        let (mut rig_app, _) = app(6);
        assert!(matches!(right_click_at(&mut rig_app, 13000.0)[..], [CommandKind::Order(Order::Move { .. })]));

        let (mut app, ids) = app(5);
        app.world_mut().insert_resource(Selection(vec![ids[5], ids[1]]));
        let cmds = right_click_at(&mut app, 10000.0);
        assert_eq!(cmds.len(), 2);
        assert!(cmds.iter().all(|c| matches!(c, CommandKind::Order(Order::Move { .. }))));
        // The cursor agrees: the ships of a mixed selection are moved, so the move cursor shows.
        app.init_resource::<crate::ui::cursor::WantedCursor>();
        app.world_mut().run_system_once(cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_move");
    }

    #[test]
    fn two_stationary_builders_give_no_order_on_a_right_click() {
        let (mut app, ids) = app(5);
        app.world_mut().insert_resource(Selection(vec![ids[5], ids[7]]));
        assert!(right_click_at_xz(&mut app, 10000.0, -350.0).is_empty());
    }

    #[test]
    fn rally_eligibility_rejects_unfinished_nonowned_and_stale_ids() {
        let (app, ids) = app(5);
        let link = app.world().resource::<SimLink>();
        let db = &app.world().resource::<ContentRes>().0;
        let mut view = link.curr().as_ref().clone();
        assert!(rally_builders(Some(db), &view, PlayerId(0), &[ids[5], ids[7]]));
        view.units.iter_mut().find(|u| u.id == ids[7]).unwrap().progress = Some(Fixed::ZERO);
        assert!(!rally_builders(Some(db), &view, PlayerId(0), &[ids[5], ids[7]]));
        assert!(!rally_builders(Some(db), &view, PlayerId(0), &[ids[2]]));
        assert!(!rally_builders(Some(db), &view, PlayerId(0), &[EntityId(u32::MAX)]));
    }

    #[test]
    fn typed_planet_context_mining_and_stationary_builders_use_resolved_roles() {
        use sta_content::PlanetRole;
        use sta_sim::view::UnitView;
        use sta_sim::world::NEUTRAL;

        let fake = fake::db();
        let mut builder = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for unit in fake.units() { builder = builder.unit(unit.clone()); }
        for weapon in fake.weapons() { builder = builder.weapon(weapon.clone()); }
        let mut background = UnitDef::new("test_background", "planet_obj");
        background.planet_role = Some(PlanetRole::Background);
        background.resource = Some(0); // metadata does not make a background mineable.
        let mut producer = UnitDef::new("test_producer", "planet");
        producer.planet_role = Some(PlanetRole::Producer);
        producer.resource = Some(0);
        producer.build.list = vec![DefRef::new("fx_miner")];
        let mut empty_producer = UnitDef::new("test_empty_producer", "planet");
        empty_producer.planet_role = Some(PlanetRole::Producer);
        let db = builder.unit(background).unit(producer).unit(empty_producer).build().unwrap();
        let make = |id, name, owner| UnitView {
            id: EntityId(id), def: db.key(name).unwrap(), owner,
            pos: SimVec3::ZERO, heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4], crew: None, energy: None, autonomy: None,
            flags: UnitFlags::SELECTABLE, progress: None,
        };
        let view = WorldView {
            units: vec![
                make(1, "fx_miner", PlayerId(0)),
                make(2, "test_background", PlayerId(0)),
                make(3, "test_producer", NEUTRAL),
                make(4, "test_empty_producer", NEUTRAL),
                make(5, "test_producer", PlayerId(0)),
            ],
            ..Default::default()
        };
        assert_eq!(context_order(Some(&db), &view, PlayerId(0), &[EntityId(1)], EntityId(2)), None);
        assert_eq!(
            context_order(Some(&db), &view, PlayerId(0), &[EntityId(1)], EntityId(3)),
            Some(Order::Mine { units: vec![EntityId(1)], node: EntityId(3) })
        );
        assert_eq!(context_order(Some(&db), &view, PlayerId(0), &[EntityId(1)], EntityId(4)), None);
        assert!(rally_builders(Some(&db), &view, PlayerId(0), &[EntityId(5)]));
        assert!(!rally_builders(Some(&db), &view, PlayerId(0), &[EntityId(2)]));
        // The sim's vote still names a rally for them (the bench uses it); the client takes none.
        assert_eq!(
            sta_sim::context::context_order(Some(&db), &view, PlayerId(0), &[EntityId(5)], EntityId(3)),
            Some(Order::Rally { builders: vec![EntityId(5)], at: Target::Entity(EntityId(3)) })
        );
        assert_eq!(context_order(Some(&db), &view, PlayerId(0), &[EntityId(5)], EntityId(3)), None);
    }
}
