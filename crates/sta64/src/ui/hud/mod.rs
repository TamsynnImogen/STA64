//! The in-game HUD (UI-04), after Refit's `ui/HUD.md`: a resources strip at
//! the top, the minimap (UI-05's radar) at the bottom left, the command grid,
//! and the info panel at three heights; notices (UI-10) hang under the strip.
//!
//! [`HudPlugin`] is for a game mode to add (INT-M3); it does nothing until a
//! [`SimLink`] exists, or a test or frame run writes [`HudFeed`] itself.
//!
//! - **Layout** ([`layout`]): panel sizes come from the race's GUI cfg
//!   ([`HudConfig::gui`], CON-12), places from HUD.md's bottom-row rules.
//! - **Info panel**: one object (low), a builder (middle), a group (high).
//!   A group tile click selects the unit (Shift removes it).
//! - **Grid**: 5 x 3 cells. The top page derives shared actions from owned
//!   units' content capabilities and current flags. The build page lists the
//!   first eligible builder's build list, through the tech
//!   tree ([`TechTree::build_options`](sta_content::TechTree::build_options))
//!   when the content has one, else the def's list; locked entries are shown
//!   disabled, hidden ones left out. A press queues `Order::Build`. A builder
//!   that moves (a construction rig) needs a site: the press arms a
//!   [`Placing`] and the next left click on the map places it.
//! - **Hosting**: the radar and the notices are moved to their HUD.md places.
//!
//! What a game mode must do for it is in the INT-M3 notes of the UI-04 PR:
//! add the plugin after `SimLinkPlugin`, [`map_input_free`] on the selection
//! and order systems, and a `SimLink` with a content resource.

// Wired into the game by INT-M3; until then only the frame runs and tests use it.
#![allow(dead_code)]

pub mod frame;
pub mod ghost;
pub mod layout;
pub mod model;
pub mod commands;
mod panels;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use sta_content::GuiLayout;
use sta_sim::command::{CommandKind, Order, Target};
use sta_sim::{ContentKey, EntityId, Vec3 as SimVec3};

pub use layout::{HudLayout, InfoHeight};
pub use panels::HudRoot;
pub use model::{Act, HudFeed, Page};
use model::{Models, TargetMode, command, models};
use panels::{GridCell, GridPanel, GroupTile, InfoPanel, ResourceStrip};

use super::notices::{Notice, NoticePlugin, NoticeRoot, Severity};
use super::radar::{RadarPlugin, RadarRoot};
use super::theme::{ButtonClicked, Theme, ThemePlugin};
use crate::app::GameSet;
use crate::input::orders::{Aim, OrderTargets, formation_moves};
use crate::input::select::{Targetable, unit_under};
use crate::ui::i18n::{tr, trf};
use sta_sim::view::UnitFlags;
use sta_sim::world::EntityKind;
use crate::sim_link::{CommandQueue, ContentRes, OrderIssued, OrderKind, Selection, SelectionChanged, SimLink};

/// What the HUD is drawn from and how it fits the screen.
#[derive(Resource, Clone)]
pub struct HudConfig {
    /// The race's GUI cfg, resolved (CON-12); `None`: stock sizes.
    pub gui: Option<GuiLayout>,
    /// Screen width over height; kept from the window when there is one.
    pub aspect: f32,
    /// Scale the UI to the primary window (`UiScale` = height / 1080). Off
    /// when something else sets the scale (the frame runs).
    pub fit_scale: bool,
    /// Print the layout when it changes (HUD.md, "The layout log").
    pub log: bool,
}

impl Default for HudConfig {
    fn default() -> Self {
        HudConfig { gui: None, aspect: 16.0 / 9.0, fit_scale: true, log: false }
    }
}

/// The layout in force.
#[derive(Resource, Clone)]
pub struct HudLayoutRes(pub HudLayout);

/// A construction craft waiting for a site: the next left click on the map
/// sends `Order::Build` with it.
#[derive(Debug, Clone, PartialEq)]
pub struct Armed {
    pub builder: EntityId,
    pub what: ContentKey,
    pub odf: String,
}

/// The HUD's own state.
#[derive(Resource, Default)]
pub struct HudState {
    pub page: Page,
    /// A build waiting for a site.
    pub placing: Option<Armed>,
    pub targeting: Option<TargetMode>,
    drawn: Option<Models>,
    height: Option<InfoHeight>,
    logged: String,
}

/// Marks a feed written by someone else (the frame runs): [`feed_from_link`] leaves it alone.
#[derive(Resource)]
pub struct ExternalFeed;

/// Whether the pointer is over a HUD panel this frame (theme units, from the
/// window cursor), for input to keep clicks off the map.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudPointer(pub bool);

/// Run condition for the map's selection and order systems: the pointer is not
/// over the HUD and no placement is waiting for the click.
pub fn map_input_free(pointer: Option<Res<HudPointer>>, state: Option<Res<HudState>>) -> bool {
    !pointer.is_some_and(|p| p.0) && !state.is_some_and(|s| s.placing.is_some() || s.targeting.is_some())
}

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ThemePlugin>() {
            app.add_plugins(ThemePlugin);
        }
        if !app.is_plugin_added::<super::sprite_font::SpriteFontPlugin>() {
            app.add_plugins(super::sprite_font::SpriteFontPlugin);
        }
        if !app.is_plugin_added::<NoticePlugin>() {
            app.add_plugins(NoticePlugin);
        }
        if !app.is_plugin_added::<RadarPlugin>() {
            app.add_plugins(RadarPlugin);
        }
        let config = app.world().get_resource::<HudConfig>().cloned().unwrap_or_default();
        app.insert_resource(HudLayoutRes(HudLayout::new(config.gui.as_ref(), config.aspect)))
            .init_resource::<HudConfig>()
            .init_resource::<UiScale>()
            .init_resource::<HudFeed>()
            .init_resource::<HudState>()
            .init_resource::<HudPointer>()
            .init_resource::<super::gui_sprites::GuiSkin>()
            .init_resource::<super::gui_sprites::GuiSprites>()
            .init_resource::<ghost::PlacementBounds>()
            .init_resource::<OrderTargets>()
            .init_resource::<CommandQueue>()
            .add_message::<OrderIssued>()
            .add_message::<SelectionChanged>()
            .init_resource::<Selection>()
            .add_systems(Startup, panels::spawn_root.after(crate::ui::radar::spawn_radar))
            .add_systems(
                Update,
                (
                    panels::reset_on_theme,
                    panels::spawn_root,
                    feed_from_link.run_if(resource_exists::<SimLink>).run_if(not(resource_exists::<ExternalFeed>)),
                    fit_screen,
                    refresh,
                    panels::update_values.before(super::theme::WidgetPaint),
                    host,
                    pointer,
                    press,
                    place_on_click.run_if(resource_exists::<Aim>),
                    ghost::update_ghost,
                    ghost::tint_ghost,
                    super::gui_sprites::paint.after(super::theme::WidgetPaint),
                )
                    .chain()
                    .in_set(GameSet::Ui)
                    .after(crate::ui::profile::ProfileSync)
                    .after(crate::ui::radar::RadarUi),
            );
    }
}

/// Copy the game's view, player, selection and content into the feed.
pub fn feed_from_link(link: Res<SimLink>, selection: Res<Selection>, content: Option<Res<ContentRes>>, mut feed: ResMut<HudFeed>) {
    feed.view = link.curr().clone();
    feed.player = link.player();
    feed.selected.clone_from(&selection.0);
    feed.content = content.map(|c| c.0.clone());
}

/// Fit the UI to the primary window and keep the layout's aspect.
fn fit_screen(
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    mut config: ResMut<HudConfig>,
    mut layout: ResMut<HudLayoutRes>,
    mut scale: ResMut<UiScale>,
) {
    if let Some(w) = window
        && w.height() > 0.0
    {
        let aspect = w.width() / w.height();
        if (config.aspect - aspect).abs() > 1e-3 {
            config.aspect = aspect;
        }
        if config.fit_scale && (scale.0 - w.height() / layout::SCREEN_H).abs() > 1e-4 {
            scale.0 = w.height() / layout::SCREEN_H;
        }
    }
    if config.is_changed() {
        layout.0 = HudLayout::new(config.gui.as_ref(), config.aspect);
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn refresh(
    mut commands: Commands,
    theme: Res<Theme>,
    feed: Res<HudFeed>,
    layout: Res<HudLayoutRes>,
    config: Res<HudConfig>,
    mut state: ResMut<HudState>,
    mut strip: Query<(Entity, &mut Node, Option<&Children>), (With<ResourceStrip>, Without<InfoPanel>, Without<GridPanel>)>,
    mut info_panel: Query<(Entity, &mut Node, Option<&Children>), (With<InfoPanel>, Without<ResourceStrip>, Without<GridPanel>)>,
    mut grid_panel: Query<(Entity, &mut Node, Option<&Children>), (With<GridPanel>, Without<ResourceStrip>, Without<InfoPanel>)>,
) {
    // The build page needs a builder; without one it is the top page.
    if state.page == Page::Build && (!commands::shared(&feed).build || model::builder(&feed).is_none()) {
        state.page = Page::Top;
    }
    if state.page == Page::Formations && !commands::shared(&feed).move_order {
        state.page = Page::Top;
    }
    let now = models(&feed, state.page, layout.0.no_resource);
    let height = now.info.height();
    let layout_changed = layout.is_changed() || theme.is_changed() || state.height != height || state.drawn.is_none();
    let drawn = state.drawn.take();
    let l = &layout.0;
    let row = l.bottom_row(height);

    if let Ok((e, mut node, kids)) = strip.single_mut() {
        if layout_changed {
            panels::place(&mut node, l.resources);
        }
        if layout_changed || drawn.as_ref().is_none_or(|d| !panels::same_resources(&d.resources, &now.resources)) {
            clear(&mut commands, kids);
            panels::fill_resources(&mut commands, &theme, e, &now.resources);
        }
    }
    if let Ok((e, mut node, kids)) = info_panel.single_mut() {
        if layout_changed {
            panels::place(&mut node, row.info);
        }
        if layout_changed || drawn.as_ref().is_none_or(|d| !panels::same_info(&d.info, &now.info)) {
            clear(&mut commands, kids);
            panels::fill_info(&mut commands, &theme, e, &now.info, Vec2::new(row.info.w, row.info.h));
        }
    }
    if let Ok((e, mut node, kids)) = grid_panel.single_mut() {
        if layout_changed {
            panels::place(&mut node, row.grid);
        }
        if layout_changed || drawn.as_ref().is_none_or(|d| d.grid != now.grid) {
            clear(&mut commands, kids);
            panels::fill_grid(&mut commands, &theme, e, &now.grid);
        }
    }
    if config.log {
        let line = log_line(l, height, state.page, &now, &row);
        if line != state.logged {
            println!("{line}");
            state.logged = line;
        }
    }
    state.height = height;
    state.drawn = Some(now);
}

fn clear(commands: &mut Commands, kids: Option<&Children>) {
    for k in kids.into_iter().flatten() {
        commands.entity(*k).despawn();
    }
}

/// The layout log: one line per change, for the bench and the frame tests.
fn log_line(l: &HudLayout, height: Option<InfoHeight>, page: Page, m: &Models, row: &layout::BottomRow) -> String {
    let r = |r: layout::R| format!("{:.0},{:.0},{:.0},{:.0}", r.x, r.y, r.w, r.h);
    let cells: Vec<String> = m.grid.iter().map(|b| format!("{}:{}", b.cell, b.label)).collect();
    format!(
        "hud layout: screen={:.0}x{:.0} info={} page={:?} resources={} minimap={} grid={} info_rect={} beside={} cells=[{}] strip=[{}]",
        l.screen.x,
        l.screen.y,
        height.map_or("none".to_owned(), |h| format!("{h:?}").to_lowercase()),
        page,
        r(l.resources),
        r(l.minimap),
        r(row.grid),
        r(row.info),
        row.grid_beside,
        cells.join(" "),
        m.resources.iter().map(|e| format!("{}={}", e.name.to_lowercase(), e.text)).collect::<Vec<_>>().join(" "),
    )
}

/// Put the radar and the notices where HUD.md has them.
fn host(
    layout: Res<HudLayoutRes>,
    mut radar: Query<&mut Node, (With<RadarRoot>, Without<NoticeRoot>)>,
    mut notices: Query<&mut Node, (With<NoticeRoot>, Without<RadarRoot>)>,
) {
    let l = &layout.0;
    for mut n in &mut radar {
        let (left, bottom) = (Val::Px(l.minimap.x), Val::Px(l.screen.y - l.minimap.bottom()));
        if n.left != left || n.bottom != bottom {
            n.left = left;
            n.bottom = bottom;
        }
    }
    for mut n in &mut notices {
        let top = Val::Px(l.notices_top());
        if n.top != top {
            n.top = top;
        }
    }
}

/// Is the window cursor over a HUD panel?
fn pointer(
    window: Option<Single<&Window, With<PrimaryWindow>>>,
    scale: Res<UiScale>,
    layout: Res<HudLayoutRes>,
    state: Res<HudState>,
    mut out: ResMut<HudPointer>,
) {
    let over = window.and_then(|w| w.cursor_position()).is_some_and(|c| {
        let p = c / scale.0.max(1e-3);
        layout.0.solid(state.height).iter().any(|r| r.contains(p))
    });
    if out.0 != over {
        out.0 = over;
    }
}

/// Presses: grid cells and group tiles.
#[allow(clippy::too_many_arguments)]
fn press(
    mut clicks: MessageReader<ButtonClicked>,
    cells: Query<&GridCell>,
    tiles: Query<&GroupTile>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    feed: Res<HudFeed>,
    mut state: ResMut<HudState>,
    mut queue: ResMut<CommandQueue>,
    mut issued: MessageWriter<OrderIssued>,
    mut selection: ResMut<Selection>,
    mut changed: MessageWriter<SelectionChanged>,
    mut notices: MessageWriter<Notice>,
    mut targets: ResMut<OrderTargets>,
) {
    for ButtonClicked(e) in clicks.read() {
        if let Ok(GroupTile(id)) = tiles.get(*e) {
            let shift = keys
                .as_ref()
                .is_some_and(|k| k.pressed(KeyCode::ShiftLeft) || k.pressed(KeyCode::ShiftRight));
            selection.0 = if shift {
                selection.0.iter().copied().filter(|s| s != id).collect()
            } else {
                vec![*id]
            };
            changed.write(SelectionChanged);
            continue;
        }
        let Ok(GridCell(act)) = cells.get(*e) else {
            continue;
        };
        match act {
            Act::OpenBuild => state.page = Page::Build,
            Act::OpenFormations if commands::shared(&feed).move_order => state.page = Page::Formations,
            Act::Back => {
                state.page = Page::Top;
                state.placing = None;
                state.targeting = None;
            }
            Act::Target(mode) if target_mode_available(&feed, *mode) => {
                state.placing = None;
                state.targeting = Some(*mode);
                notices.write(Notice::new(
                    Severity::Info,
                    trf("{0}: click the map (Esc cancels)", &[&tr(&format!("{mode:?}"))]),
                ));
            }
            Act::Build {
                builder,
                what,
                odf,
                site: true,
            } => {
                if !commands::shared(&feed).build
                    || model::builder(&feed).is_none_or(|u| u.id != *builder)
                {
                    continue;
                }
                state.targeting = None;
                state.placing = Some(Armed {
                    builder: *builder,
                    what: *what,
                    odf: odf.clone(),
                });
                notices.write(Notice::new(
                    Severity::Info,
                    trf("Place {0}: click the map (Esc cancels)", &[odf]),
                ));
            }
            other => {
                if let Some((kind, units)) = command(&feed, other) {
                    if matches!(&kind, CommandKind::Order(Order::Stop { .. })) { clear_order_targets(&mut targets, &units); }
                    if let CommandKind::Order(o) = &kind
                        && let Some(k) = OrderKind::of(o)
                    {
                        issued.write(OrderIssued { units, kind: k });
                    }
                    queue.0.push(kind);
                }
            }
        }
    }
}

/// The click that places an armed build: the point on the ground under the
/// pointer, at the builder's height.
#[allow(clippy::too_many_arguments)]
fn place_on_click(
    mut state: ResMut<HudState>,
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    aim: Res<Aim>,
    pointer: Res<HudPointer>,
    feed: Res<HudFeed>,
    mut queue: ResMut<CommandQueue>,
    mut issued: MessageWriter<OrderIssued>,
    units: Targetable,
    mut targets: ResMut<OrderTargets>,
    bounds: Res<ghost::PlacementBounds>,
    mut notices: MessageWriter<Notice>,
) {
    if keys
        .as_ref()
        .is_some_and(|k| k.just_pressed(KeyCode::Escape))
        || buttons
            .as_ref()
            .is_some_and(|b| b.just_pressed(MouseButton::Right))
    {
        state.placing = None;
        state.targeting = None;
        return;
    }
    if let Some(mode) = state.targeting {
        if !target_mode_available(&feed, mode) {
            state.targeting = None;
            return;
        }
        let Some(buttons) = buttons.as_ref() else {
            return;
        };
        if !buttons.just_pressed(MouseButton::Left) || pointer.0 {
            return;
        }
        let Some(a) = aim.0 else { return };
        let target = unit_under(a.cursor, a.ray, &units);
        let own = feed.own_selection();
        let heights: Vec<f32> = own.iter().filter_map(|id| feed.view.unit(*id).map(|u| u.pos.y.to_f32())).collect();
        let y = if heights.is_empty() { 0.0 } else { heights.iter().sum::<f32>() / heights.len() as f32 };
        let point = ground_point(a.ray, y);
        if let Some((orders, ids)) = target_order(&feed, mode, target, point) {
            if let Some(kind) = orders.first().and_then(OrderKind::of) {
                issued.write(OrderIssued { units: ids.clone(), kind });
            }
            // A new order replaces the old destination line (a move sets its own).
            clear_order_targets(&mut targets, &ids);
            for order in orders {
                record_move_target(&mut targets, &order);
                queue.0.push(CommandKind::Order(order));
            }
            state.targeting = None;
        }
        return;
    }
    let Some(armed) = state.placing.clone() else {
        return;
    };
    if !commands::shared(&feed).build || model::builder(&feed).is_none_or(|u| u.id != armed.builder)
    {
        state.placing = None;
        return;
    }
    let Some(buttons) = buttons else { return };
    if !buttons.just_pressed(MouseButton::Left) || pointer.0 {
        return;
    }
    let (Some(a), Some(b)) = (aim.0, feed.view.unit(armed.builder)) else {
        return;
    };
    let y = b.pos.y.to_f32();
    let dir = *a.ray.direction;
    if dir.y.abs() < 1e-4 {
        return;
    }
    let t = (y - a.ray.origin.y) / dir.y;
    if t <= 0.0 {
        return;
    }
    // Render space to the sim's: z runs the other way.
    let hit = a.ray.origin + dir * t;
    let at = SimVec3::from_f32([hit.x, y, -hit.z]);
    if !ghost::affordable(&feed, &armed) {
        notices.write(Notice::new(Severity::Warn, tr("Cannot build: not enough resources")));
        return;
    }
    // The sim's own rule, as the ghost shows it: a refused site keeps the
    // placement armed so the click can be tried again elsewhere.
    if let Err(e) = ghost::site_status(&feed, bounds.0.as_ref(), &armed, at) {
        notices.write(Notice::new(Severity::Warn, tr(ghost::refusal(e))));
        return;
    }
    // The new order's target replaces the destination line of the last one.
    targets.0.insert(armed.builder, at);
    queue.0.push(CommandKind::Order(Order::Build {
        builder: armed.builder,
        what: armed.what,
        at: Some(at),
    }));
    issued.write(OrderIssued {
        units: vec![armed.builder],
        kind: OrderKind::Build,
    });
    state.placing = None;
}

fn target_mode_available(feed: &HudFeed, mode: TargetMode) -> bool {
    let c = commands::shared(feed);
    match mode {
        TargetMode::Move => c.move_order,
        TargetMode::Attack => c.attack,
        TargetMode::Mine => c.mine,
        TargetMode::Rally => c.rally,
    }
}

/// Resolve a grid target against the current view, after capability and ownership checks.
fn target_order(
    feed: &HudFeed,
    mode: TargetMode,
    target: Option<EntityId>,
    point: Option<SimVec3>,
) -> Option<(Vec<Order>, Vec<EntityId>)> {
    if !target_mode_available(feed, mode) { return None; }
    let ids = feed.own_selection();
    let orders = match mode {
        TargetMode::Move => {
            let picked: Vec<(EntityId, Vec3)> = ids.iter().filter_map(|id| feed.view.unit(*id).map(|u| (*id, Vec3::from(u.pos.to_f32())))).collect();
            if picked.is_empty() { return None; }
            let centre = picked.iter().fold(Vec3::ZERO, |a, (_, pos)| a + *pos) / picked.len() as f32;
            let dest = Vec3::from(point?.to_f32());
            formation_moves(&picked, centre, dest).into_iter().map(|(id, to)| Order::Move { units: vec![id], to: SimVec3::from_f32(to.to_array()), queue: false }).collect()
        }
        TargetMode::Attack => {
            let id = target?;
            let u = feed.view.unit(id)?;
            if u.flags.contains(UnitFlags::DYING) || u.hull[1] <= sta_sim::Fixed::ZERO || !enemy_target(feed, u.owner) { return None; }
            vec![Order::Attack { units: ids.clone(), target: id }]
        }
        TargetMode::Mine => {
            let id = target?;
            let u = feed.view.unit(id)?;
            let db = feed.content.as_deref()?;
            let def = db.unit(u.def)?;
            if u.flags.contains(UnitFlags::DYING) || !EntityKind::mineable_def(def) { return None; }
            vec![Order::Mine { units: ids.clone(), node: id }]
        }
        TargetMode::Rally => {
            let at = match target.and_then(|id| feed.view.unit(id).filter(|u| !u.flags.contains(UnitFlags::DYING)).map(|_| id)) {
                Some(id) => Target::Entity(id),
                None => Target::Point(point?),
            };
            vec![Order::Rally { builders: ids.clone(), at }]
        }
    };
    Some((orders, ids))
}

fn enemy_target(feed: &HudFeed, owner: sta_sim::PlayerId) -> bool {
    if owner == feed.player || !owner.is_player() { return false; }
    let mine = feed.view.player(feed.player);
    let theirs = feed.view.player(owner);
    if let (Some(m), Some(t)) = (mine, theirs)
        && m.team == t.team { return false; }
    let relation = mine.and_then(|m| m.relations.iter().find(|(id, _)| *id == owner).map(|(_, relation)| *relation))
        .or_else(|| theirs.and_then(|t| t.relations.iter().find(|(id, _)| *id == feed.player).map(|(_, relation)| *relation)));
    match relation {
        Some(sta_sim::view::Relation::Enemy) | None => true,
        Some(_) => false,
    }
}

fn record_move_target(targets: &mut OrderTargets, order: &Order) {
    if let Order::Move { units, to, .. } = order {
        for id in units { targets.0.insert(*id, *to); }
    }
}

fn clear_order_targets(targets: &mut OrderTargets, units: &[EntityId]) {
    for id in units { targets.0.remove(id); }
}

fn ground_point(ray: Ray3d, y: f32) -> Option<SimVec3> {
    let dir = *ray.direction;
    if dir.y.abs() < 1e-4 {
        return None;
    }
    let t = (y - ray.origin.y) / dir.y;
    if t <= 0.0 {
        return None;
    }
    let hit = ray.origin + dir * t;
    Some(SimVec3::from_f32([hit.x, y, -hit.z]))
}

#[cfg(test)]
mod target_tests {
    use super::*;
    use sta_sim::view::{PlayerView, Relation, UnitView};
    use sta_sim::{Fixed, PlayerId, TeamId, WorldView};
    use std::sync::Arc;

    fn feed(selected: &[u32]) -> HudFeed {
        let db = sta_content::fake::db();
        let make = |id, odf, owner| UnitView {
            id: EntityId(id), def: db.key(odf).unwrap(), owner: PlayerId(owner),
            pos: SimVec3::ZERO, heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4],
            flags: UnitFlags::SELECTABLE, progress: None,
        };
        HudFeed {
            view: Arc::new(WorldView { units: vec![make(1, "fx_cruiser", 1), make(2, "fx_miner", 1), make(3, "fx_yard", 1), make(4, "fx_cruiser", 2), make(5, "fx_moon", 255)], ..Default::default() }),
            player: PlayerId(1), selected: selected.iter().copied().map(EntityId).collect(), content: Some(Arc::new(db)),
        }
    }

    fn player(id: u8, team: u8, relation: Option<Relation>) -> PlayerView {
        PlayerView { id: PlayerId(id), team: TeamId(team), resources: Vec::new(), officers: [Fixed::ZERO; 2], eliminated: false,
            relations: relation.map(|r| vec![(PlayerId(if id == 1 { 2 } else { 1 }), r)]).unwrap_or_default() }
    }

    #[test]
    fn target_actions_make_supported_orders() {
        let p = Some(SimVec3::from_f32([10.0, 0.0, 20.0]));
        let f = feed(&[1]);
        assert!(matches!(target_order(&f, TargetMode::Move, None, p), Some((orders, _)) if matches!(orders.as_slice(), [Order::Move { units, to, .. }] if units == &[EntityId(1)] && Some(*to) == p)));
        assert!(matches!(target_order(&f, TargetMode::Attack, Some(EntityId(4)), p), Some((orders, _)) if matches!(orders.as_slice(), [Order::Attack { target: EntityId(4), .. }])));
        assert!(target_order(&f, TargetMode::Attack, Some(EntityId(1)), p).is_none());
        assert!(target_order(&f, TargetMode::Attack, Some(EntityId(99)), p).is_none());
        let f = feed(&[2]);
        assert!(matches!(target_order(&f, TargetMode::Mine, Some(EntityId(5)), p), Some((orders, _)) if matches!(orders.as_slice(), [Order::Mine { node: EntityId(5), .. }])));
        assert!(target_order(&f, TargetMode::Mine, Some(EntityId(4)), p).is_none());
        let f = feed(&[3]);
        assert!(matches!(target_order(&f, TargetMode::Rally, None, p), Some((orders, _)) if matches!(orders.as_slice(), [Order::Rally { builders, at: Target::Point(_) }] if builders == &[EntityId(3)])));
        assert!(matches!(target_order(&f, TargetMode::Rally, Some(EntityId(4)), p), Some((orders, _)) if matches!(orders.as_slice(), [Order::Rally { at: Target::Entity(EntityId(4)), .. }])));
    }

    #[test]
    fn move_keeps_offsets_and_updates_and_clears_waypoint_cache() {
        let mut f = feed(&[1, 2]);
        Arc::make_mut(&mut f.view).units[1].pos = SimVec3::from_f32([100.0, 0.0, 0.0]);
        let (orders, ids) = target_order(&f, TargetMode::Move, None, Some(SimVec3::from_f32([1000.0, 0.0, 0.0]))).unwrap();
        assert_eq!(ids, [EntityId(1), EntityId(2)]);
        assert_eq!(orders, [
            Order::Move { units: vec![EntityId(1)], to: SimVec3::from_f32([950.0, 0.0, 0.0]), queue: false },
            Order::Move { units: vec![EntityId(2)], to: SimVec3::from_f32([1050.0, 0.0, 0.0]), queue: false },
        ]);
        let mut cache = OrderTargets::default();
        for order in &orders { record_move_target(&mut cache, order); }
        assert_eq!(cache.0.get(&EntityId(1)), Some(&SimVec3::from_f32([950.0, 0.0, 0.0])));
        assert_eq!(cache.0.get(&EntityId(2)), Some(&SimVec3::from_f32([1050.0, 0.0, 0.0])));
        clear_order_targets(&mut cache, &ids);
        assert!(cache.0.is_empty());
    }

    #[test]
    fn attack_rejects_known_allies_and_neutral_relations() {
        let mut f = feed(&[1]);
        let p = Some(SimVec3::ZERO);
        Arc::make_mut(&mut f.view).players = vec![player(1, 1, None), player(2, 1, None)];
        assert!(target_order(&f, TargetMode::Attack, Some(EntityId(4)), p).is_none(), "same team");
        Arc::make_mut(&mut f.view).players = vec![player(1, 1, Some(Relation::Ally)), player(2, 2, None)];
        assert!(target_order(&f, TargetMode::Attack, Some(EntityId(4)), p).is_none(), "ally relation");
        Arc::make_mut(&mut f.view).players = vec![player(1, 1, Some(Relation::Neutral)), player(2, 2, None)];
        assert!(target_order(&f, TargetMode::Attack, Some(EntityId(4)), p).is_none(), "neutral relation");
        Arc::make_mut(&mut f.view).players = vec![player(1, 1, Some(Relation::Enemy)), player(2, 2, None)];
        assert!(target_order(&f, TargetMode::Attack, Some(EntityId(4)), p).is_some());
        Arc::make_mut(&mut f.view).units.iter_mut().find(|u| u.id == EntityId(4)).unwrap().owner = sta_sim::PlayerId(253);
        assert!(target_order(&f, TargetMode::Attack, Some(EntityId(4)), p).is_none(), "nonplayer neutral");
    }

    #[test]
    fn stale_and_disabled_selection_cannot_emit_target_orders() {
        let p = Some(SimVec3::ZERO);
        let mut f = feed(&[1]);
        Arc::make_mut(&mut f.view).units[0].flags = UnitFlags::SELECTABLE.with(UnitFlags::ENGINES_OFF, true);
        assert!(target_order(&f, TargetMode::Move, None, p).is_none());
        f.selected = vec![EntityId(99)];
        assert!(target_order(&f, TargetMode::Attack, Some(EntityId(4)), p).is_none());
        f.selected = vec![EntityId(3)];
        Arc::make_mut(&mut f.view).units[2].owner = PlayerId(2);
        assert!(target_order(&f, TargetMode::Rally, None, p).is_none());
    }
}

#[cfg(test)]
mod tests;
