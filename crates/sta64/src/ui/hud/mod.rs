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
//!   that is not a producer station (a construction rig, a craft by class
//!   label) needs a site: the press arms a [`Placing`] and the next left
//!   click on the map places it. A producer station (starbase, shipyard, by
//!   class label, not by speed) queues in place and takes a rally point.
//! - **Hosting**: the radar and the notices are moved to their HUD.md places.
//!
//! What a game mode must do for it is in the INT-M3 notes of the UI-04 PR:
//! add the plugin after `SimLinkPlugin`, [`map_input_free`] on the selection
//! and order systems, and a `SimLink` with a content resource.

// Wired into the game by INT-M3; until then only the frame runs and tests use it.
#![allow(dead_code)]

mod cinematic;
mod footprint;
pub(crate) mod object_footprints;
pub mod frame;
pub mod instance_names;
pub mod ghost;
pub mod layout;
pub mod model;
pub mod commands;
mod panels;
pub mod stock;
mod status;
mod status_gpu;

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
use crate::input::select::{Targetable, target_seen, unit_under_where};
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
    /// How the stock-look HUD sits on a window whose aspect differs from its sheet (the player's
    /// `hud_layout` setting, kept in step by `fit_screen`). The LCARS HUD ignores it.
    pub fit: stock::layout::HudFit,
    /// Scale the UI to the primary window (`UiScale` = height / 1080). Off
    /// when something else sets the scale (the frame runs).
    pub fit_scale: bool,
    /// Print the layout when it changes (HUD.md, "The layout log").
    pub log: bool,
}

impl Default for HudConfig {
    fn default() -> Self {
        HudConfig { gui: None, aspect: 16.0 / 9.0, fit: stock::layout::HudFit::Centred, fit_scale: true, log: false }
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
        // The stock button panel (UI-48) writes these; their own plugins register them too.
        app.add_message::<crate::input::select::CameraFocus>()
            .add_message::<crate::ui::ingame_menu::OpenInGameMenu>();
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
            .init_resource::<crate::ui::cursor::WantedCursor>()
            .init_resource::<crate::input::hover::HoveredUnit>()
            .init_resource::<stock::StockState>()
            .init_resource::<stock::buttons::ShowMore>()
            .init_resource::<status::StatusFeed>()
            .init_resource::<status_gpu::StatusUiState>()
            .init_resource::<cinematic::CinematicState>()
            .init_resource::<super::gui_sprites::GuiSkin>()
            .init_resource::<super::gui_sprites::GuiSprites>()
            .init_resource::<ghost::PlacementBounds>()
            .init_resource::<OrderTargets>()
            .init_resource::<CommandQueue>()
            .add_message::<OrderIssued>()
            .add_message::<SelectionChanged>()
            .init_resource::<Selection>()
            .add_systems(OnEnter(crate::app::AppState::Loading), stock::buttons::reset_more)
            .add_systems(Startup, panels::spawn_root.after(crate::ui::radar::spawn_radar))
            .add_systems(
                Update,
                (
                    panels::reset_on_theme,
                    panels::spawn_root,
                    feed_from_link.run_if(resource_exists::<SimLink>).run_if(not(resource_exists::<ExternalFeed>)),
                    fit_screen,
                    refresh,
                    (status::sync, stock::refresh, stock::update_values, stock::update_hull_tints, status_gpu::refresh, status_gpu::update_values).chain(),
                    stock::active,
                    (stock::navigate, stock::buttons::press, stock::buttons::apply_more).chain(),
                    panels::update_values.before(super::theme::WidgetPaint),
                    host,
                    pointer,
                    crate::input::hover::track.run_if(not(resource_exists::<ExternalFeed>)),
                    stock::infobar::update,
                    press.run_if(not(crate::ui::presentation::cutscene_running)),
                    place_on_click.run_if(resource_exists::<Aim>).run_if(not(crate::ui::presentation::cutscene_running)).run_if(crate::cheats::input::game_input_free),
                    armed_cursor.run_if(resource_exists::<Aim>)
                        .run_if(not(crate::ui::presentation::cutscene_running))
                        .run_if(not(resource_exists::<crate::frontend::scene::MenuScene>))
                        .run_if(crate::ui::ingame_menu::menu_closed)
                        .run_if(crate::cheats::input::game_input_free),
                    ghost::update_ghost,
                    ghost::tint_ghost,
                    (ghost::sync_footprints, object_footprints::sync_object_footprints).chain(),
                    super::gui_sprites::paint.after(super::theme::WidgetPaint),
                )
                    .chain()
                    .in_set(GameSet::Ui)
                    .after(crate::ui::profile::ProfileSync)
                    .after(crate::ui::radar::RadarUi),
            )
            .add_systems(
                Update,
                (cinematic::build, cinematic::show)
                    .chain()
                    .in_set(GameSet::Ui)
                    .after(stock::refresh)
                    .after(super::gui_sprites::paint),
            );
    }
}

/// Copy the game's view, player, selection and content into the feed.
pub fn feed_from_link(link: Res<SimLink>, selection: Res<Selection>, content: Option<Res<ContentRes>>, names: Option<Res<instance_names::InstanceNames>>, mut feed: ResMut<HudFeed>) {
    feed.view = link.curr().clone();
    feed.names = names.map(|n| n.clone()).unwrap_or_default();
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
    settings: Option<Res<crate::frontend::screens::Settings>>,
) {
    if let Some(s) = settings
        && config.fit != s.hud_fit
    {
        config.fit = s.hud_fit;
    }
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
    profile: Option<Res<crate::ui::profile::UiProfile>>,
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
    if stock::enabled(profile.as_deref(), &config) {
        state.drawn = None; state.height = None;
        return;
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
            panels::fill_info(&mut commands, &theme, e, &now.info, Vec2::new(row.info.w, row.info.h), &feed);
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
    stock: Res<stock::StockState>,
    mut radar: Query<&mut Node, (With<RadarRoot>, Without<NoticeRoot>)>,
    mut notices: Query<&mut Node, (With<NoticeRoot>, Without<RadarRoot>)>,
) {
    let l = &layout.0;
    for mut n in &mut radar {
        if let Some(layout) = &stock.layout {
            if let Some(r) = layout.panel("minimap") {
                n.left = Val::Px(r.x); n.top = Val::Px(r.y);
                n.bottom = Val::Auto; n.right = Val::Auto;
            }
        } else {
            n.left = Val::Px(l.minimap.x); n.bottom = Val::Px(l.screen.y - l.minimap.bottom());
            n.top = Val::Auto; n.right = Val::Auto;
        }
    }
    for mut n in &mut notices {
        let strip = stock.layout.as_ref().and_then(|g| stock::resource_panels(g).into_iter().map(|r| r.bottom()).reduce(f32::max));
        let top = Val::Px(strip.map_or_else(|| l.notices_top(), |bottom| bottom + 8.0));
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
    stock: Res<stock::StockState>,
    status: Res<status_gpu::StatusUiState>,
    mut out: ResMut<HudPointer>,
) {
    let over = window.and_then(|w| w.cursor_position()).is_some_and(|c| {
        let p = c / scale.0.max(1e-3);
        if status.fallback_card().is_some_and(|r| r.contains(p)) { true }
        else if stock.layout.is_some() { stock.solid.iter().any(|r| r.contains(p)) }
        else { layout.0.solid(state.height).iter().any(|r| r.contains(p)) }
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
    bindings: Option<Res<crate::input::keys::KeyBindings>>,
    menu: Option<Res<crate::ui::ingame_menu::InGameMenu>>,
    scene: Option<Res<crate::frontend::scene::MenuScene>>,
    feed: Res<HudFeed>,
    mut state: ResMut<HudState>,
    mut queue: ResMut<CommandQueue>,
    mut issued: MessageWriter<OrderIssued>,
    mut selection: ResMut<Selection>,
    mut changed: MessageWriter<SelectionChanged>,
    mut notices: MessageWriter<Notice>,
    mut targets: ResMut<OrderTargets>,
    cheat: Option<Res<crate::cheats::input::CheatInput>>,
) {
    if cheat.is_some_and(|c| c.open || c.blocked_this_frame) {
        clicks.read().for_each(drop);
        return;
    }
    let pd_hotkey_slot = (scene.is_none()
        && menu.as_deref().is_none_or(|menu| !menu.open)
        && keys.as_deref().zip(bindings.as_deref()).is_some_and(|(keys, bindings)|
            bindings.just_pressed(crate::input::keys::Action::PointDefence, keys)))
        .then(|| commands::shared(&feed).specials.into_iter()
            .find(|(_, kind)| *kind == sta_sim::specials::SpecialKind::PointDefence)
            .map(|(slot, _)| slot))
        .flatten();
    let mut clicked_pd_slot = None;
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
            Act::OpenNativeMenu(id) => state.page = Page::Native(*id),
            Act::OpenOrders => state.page = Page::Orders,
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
            Act::Target(TargetMode::Transport) => {
                // Greyed button pressed (or the selection changed): say why nothing was armed.
                let why = if commands::shared(&feed).transport_family { "Cannot transport: no crew to spare" } else { "Transporters are not available in this game yet" };
                notices.write(Notice::new(Severity::Warn, tr(why)));
            }
            Act::Target(TargetMode::TransportAttack) => {
                notices.write(Notice::new(Severity::Warn, tr("Cannot transport attack: no eligible crew to spare")));
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
                if let Act::Special(slot) = other
                    && Some(*slot) == pd_hotkey_slot
                {
                    clicked_pd_slot = Some(*slot);
                }
                issue_command(&feed, other, &mut queue, &mut issued, &mut targets);
            }
        }
    }
    if let Some(slot) = pd_hotkey_slot
        && clicked_pd_slot != Some(slot)
    {
        issue_command(&feed, &Act::Special(slot), &mut queue, &mut issued, &mut targets);
    }
}

fn issue_command(
    feed: &HudFeed,
    act: &Act,
    queue: &mut CommandQueue,
    issued: &mut MessageWriter<OrderIssued>,
    targets: &mut OrderTargets,
) {
    if let Some((kind, units)) = command(feed, act) {
        if matches!(&kind, CommandKind::Order(Order::Stop { .. })) { clear_order_targets(targets, &units); }
        if let CommandKind::Order(o) = &kind
            && let Some(k) = OrderKind::of(o)
        {
            issued.write(OrderIssued { units, kind: k });
        }
        queue.0.push(kind);
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
    local: Option<Res<crate::render::units::LocalPlayer>>,
    mut targets: ResMut<OrderTargets>,
    bounds: Res<ghost::PlacementBounds>,
    placement: ghost::PlacementInputs,
    mut notices: MessageWriter<Notice>,
    profile: Option<Res<crate::ui::profile::UiProfile>>,
    strings: Option<Res<crate::ui::i18n::Strings>>,
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
        if let Some((orders, ids)) = order_at_aim(&feed, mode, a, &units, local.as_deref().and_then(|l| l.0)) {
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
    let Some(at) = ground_point(a.ray, b.pos.y.to_f32()) else { return };
    if !ghost::affordable(&feed, &armed) {
        let text = stock::a1::costs::refusal(&feed, profile.as_deref(), strings.as_deref(), armed.what)
            .unwrap_or_else(|| tr("Cannot build: not enough resources"));
        notices.write(Notice::new(Severity::Warn, text));
        return;
    }
    // The sim's own rule, as the ghost shows it: a refused site keeps the
    // placement armed so the click can be tried again elsewhere.
    let (site_at, order) = match placement.order(&feed, bounds.0.as_ref(), &armed, at) {
        Ok(r) => r,
        Err(e) => {
            notices.write(Notice::new(Severity::Warn, tr(ghost::refusal(e))));
            return;
        }
    };
    // The new order's target replaces the destination line of the last one: the site the ghost shows.
    targets.0.insert(armed.builder, site_at);
    queue.0.push(CommandKind::Order(order));
    issued.write(OrderIssued {
        units: vec![armed.builder],
        kind: OrderKind::Build,
    });
    state.placing = None;
}

/// The same picked unit, visibility gate and ground height used by a left click and its cursor.
fn order_at_aim(
    feed: &HudFeed,
    mode: TargetMode,
    aim: crate::input::select::AimState,
    units: &Targetable,
    local: Option<sta_sim::PlayerId>,
) -> Option<(Vec<Order>, Vec<EntityId>)> {
    let target = unit_under_where(aim.cursor, aim.ray, units, |t| target_seen(&feed.view, local, t));
    let own = feed.own_selection();
    let heights: Vec<f32> = own.iter().filter_map(|id| feed.view.unit(*id).map(|u| u.pos.y.to_f32())).collect();
    let y = if heights.is_empty() { 0.0 } else { heights.iter().sum::<f32>() / heights.len() as f32 };
    target_order(feed, mode, target, ground_point(aim.ray, y))
}

fn armed_cursor_name(mode: TargetMode, valid: bool) -> &'static str {
    match (mode, valid) {
        (TargetMode::Move, true) => "c_move",
        (TargetMode::Move, false) => "c_nomove",
        (TargetMode::Attack, true) => "c_attack",
        (TargetMode::Attack, false) => "c_noattack",
        (TargetMode::Mine, true) => "c_mining",
        (TargetMode::Mine, false) => "c_nomining",
        (TargetMode::Colonize, true) => "c_colonize",
        (TargetMode::Colonize, false) => "c_nocolonize",
        (TargetMode::Transport, true) => "c_transport",
        (TargetMode::Transport, false) => "c_notransport",
        (TargetMode::TransportAttack, true) => "c_capture",
        (TargetMode::TransportAttack, false) => "c_nocapture",
        (TargetMode::Repair, true) => "c_repair",
        (TargetMode::Repair, false) => "c_norepair",
        // The installs have c_decommission and no refusal art of its own.
        (TargetMode::Decommission, true) => "c_decommission",
        (TargetMode::Decommission, false) => "c_nomove",
        // Installed tables have c_rally but no c_norally; use the move refusal art.
        (TargetMode::Rally, true) => "c_rally",
        (TargetMode::Rally, false) => "c_nomove",
    }
}

/// Armed picks take precedence over the context cursor, which ran in GameSet::Input.
#[allow(clippy::too_many_arguments)]
fn armed_cursor(
    state: Res<HudState>,
    aim: Res<Aim>,
    pointer: Res<HudPointer>,
    feed: Res<HudFeed>,
    units: Targetable,
    local: Option<Res<crate::render::units::LocalPlayer>>,
    bounds: Res<ghost::PlacementBounds>,
    placement: ghost::PlacementInputs,
    mut wanted: ResMut<crate::ui::cursor::WantedCursor>,
) {
    if state.targeting.is_none() && state.placing.is_none() { return; }
    if pointer.0 { wanted.0 = crate::ui::cursor::DEFAULT; return; }
    let Some(a) = aim.0 else { wanted.0 = crate::ui::cursor::DEFAULT; return };
    if let Some(mode) = state.targeting {
        wanted.0 = armed_cursor_name(mode, order_at_aim(&feed, mode, a, &units, local.as_deref().and_then(|l| l.0)).is_some());
    } else if let Some(armed) = state.placing.as_ref() {
        let valid = feed.view.unit(armed.builder)
            .and_then(|b| ground_point(a.ray, b.pos.y.to_f32()))
            .is_some_and(|at| ghost::affordable(&feed, armed)
                && placement.order(&feed, bounds.0.as_ref(), armed, at).is_ok());
        wanted.0 = if valid { "c_build" } else { "c_nobuild" };
    }
}

fn target_mode_available(feed: &HudFeed, mode: TargetMode) -> bool {
    let c = commands::shared(feed);
    match mode {
        TargetMode::Move => c.move_order,
        TargetMode::Attack => c.attack,
        TargetMode::Mine => c.mine,
        TargetMode::Rally => c.rally,
        TargetMode::Colonize => c.colonize,
        TargetMode::Transport => c.transport_ready,
        TargetMode::TransportAttack => c.transport_attack_ready,
        TargetMode::Repair => c.repair,
        TargetMode::Decommission => c.decommission,
    }
}

/// For the headless mission run: the Armada I rail's transport button for `feed`'s selection (its look,
/// `None` without the button) and the orders its pick sends on a click at `target`.
#[cfg(test)]
pub(crate) fn a1_rail_transport(feed: &HudFeed, target: EntityId) -> (Option<model::Look>, Option<Vec<Order>>) {
    let look = stock::a1::speed::buttons(feed, None).first().map(|b| b.look);
    (look, target_order(feed, TargetMode::Transport, Some(target), None).map(|(orders, _)| orders))
}

/// Resolve a grid target against the current view, after capability and ownership checks.
fn target_order(
    feed: &HudFeed,
    mode: TargetMode,
    target: Option<EntityId>,
    point: Option<SimVec3>,
) -> Option<(Vec<Order>, Vec<EntityId>)> {
    if !target_mode_available(feed, mode) { return None; }
    let mut ids = feed.own_selection();
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
            let miners = commands::miners(feed, &ids);
            if miners.is_empty() { return None; }
            // Only the miners take part: they are the units the order is reported for and whose targets are cleared.
            ids = miners.clone();
            vec![Order::Mine { units: miners, node: id }]
        }
        TargetMode::Colonize => {
            let planet = target?;
            if !colonizable_by(feed, planet) { return None; }
            let ships = commands::colony_ships(feed, &ids);
            if ships.is_empty() { return None; }
            // Only the colony ships take part, as with Mine.
            ids = ships.clone();
            vec![Order::Colonize { units: ships, planet }]
        }
        TargetMode::Transport | TargetMode::TransportAttack => {
            let id = target?;
            if !transport_target(feed, id) { return None; }
            let attack = mode == TargetMode::TransportAttack;
            // The target is never its own carrier (the sim drops it too); nothing left, nothing to send.
            let carriers: Vec<EntityId> = commands::transporters(feed, &ids, attack)
                .into_iter().filter(|c| *c != id).collect();
            if carriers.is_empty() { return None; }
            // The sim checks range, shields and the target's side for both commands.
            ids = carriers.clone();
            vec![Order::Transport { units: carriers, target: id, attack }]
        }
        TargetMode::Repair => {
            // SIM-41: repair ships aimed at a ship that is not a yard repair it with their beam.
            if let (Some(t), Some(db)) = (target, feed.content.as_deref())
                && !repair_yard(feed, t)
            {
                let rships: Vec<EntityId> = ids.iter().copied().filter(|&id| id != t && feed.view.unit(id).and_then(|u| db.unit(u.def)).is_some_and(|d| d.class.0.eq_ignore_ascii_case("repairship"))).collect();
                if !rships.is_empty() {
                    ids = rships.clone();
                    return Some((vec![Order::Repair { units: rships, yard: Some(t) }], ids));
                }
            }
            let ships = commands::repairers(feed, &ids);
            if ships.is_empty() { return None; }
            // A yard under the cursor is the one asked for; anywhere else the sim picks the best (SIM-40).
            // Whether the ship needs repair and the yard repairs is the sim's to judge.
            let yard = target.filter(|&id| repair_yard(feed, id));
            if target.is_some() && yard.is_none() && point.is_none() { return None; }
            ids = ships.clone();
            vec![Order::Repair { units: ships, yard }]
        }
        TargetMode::Decommission => {
            // A yard under the cursor is the one asked for; anywhere else the sim picks, or recycles in place.
            let own: Vec<EntityId> = ids.iter().copied().filter(|&id| feed.view.unit(id).is_some_and(|u| u.owner == feed.player)).collect();
            if own.is_empty() { return None; }
            let yard = target.filter(|&id| decommission_yard(feed, id));
            if target.is_some() && yard.is_none() && point.is_none() { return None; }
            ids = own.clone();
            vec![Order::Decommission { units: own, yard }]
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

/// A yard the HUD may name in `Order::Decommission`: a live, finished station of the player's own whose ODF
/// says `decommissionFacility = 1` (the sim's gate, SIM-45, read from the view).
fn decommission_yard(feed: &HudFeed, id: EntityId) -> bool {
    let (Some(db), Some(y)) = (feed.content.as_deref(), feed.view.unit(id)) else { return false };
    !y.flags.contains(UnitFlags::DYING)
        && y.hull[0] > sta_sim::Fixed::ZERO
        && y.progress.is_none_or(|p| p >= sta_sim::Fixed::ONE)
        && y.owner == feed.player
        && db.unit(y.def).is_some_and(|d| EntityKind::of_def(d) == EntityKind::Station && d.repair.decommission_facility == Some(true))
}

/// A yard the HUD may name in `Order::Repair`: a live, finished station of the player's own team whose ODF
/// says `repairFacility = 1` (the sim's yard gate, SIM-40, read from the view).
fn repair_yard(feed: &HudFeed, id: EntityId) -> bool {
    let (Some(db), Some(y)) = (feed.content.as_deref(), feed.view.unit(id)) else { return false };
    let mine = feed.view.player(feed.player).map(|p| p.team);
    let team = |p: sta_sim::PlayerId| feed.view.player(p).map(|v| v.team);
    !y.flags.contains(UnitFlags::DYING)
        && y.hull[0] > sta_sim::Fixed::ZERO
        && y.progress.is_none_or(|p| p >= sta_sim::Fixed::ONE)
        && (y.owner == feed.player || (mine.is_some() && team(y.owner) == mine))
        && db.unit(y.def).is_some_and(|d| EntityKind::of_def(d) == EntityKind::Station && d.repair.repair_facility == Some(true))
}

/// The sim's target gate for `Order::Transport`, read from the view: a live ship or station with a crew
/// capacity of at least one that is not a planet. Whose it is, how far and its shields are decided by the sim.
fn transport_target(feed: &HudFeed, target: EntityId) -> bool {
    let (Some(db), Some(t)) = (feed.content.as_deref(), feed.view.unit(target)) else { return false };
    let Some(def) = db.unit(t.def) else { return false };
    !t.flags.contains(UnitFlags::DYING)
        && t.hull[0] > sta_sim::Fixed::ZERO
        && matches!(EntityKind::of_def(def), EntityKind::Craft | EntityKind::Station)
        && def.planet_role.is_none()
        && t.crew.is_some_and(|c| c[1] >= sta_sim::Fixed::ONE)
}

/// The sim's `Order::Colonize` gate on the planet, read from the view: a live
/// colonizable Producer planet, unowned or owned by the player's own seat (a
/// teammate's, an ally's or an enemy's is refused), with no colonizer yet.
/// Evidence: the sim's gate (SIM-30), unchanged and shared with the HUD; read from the sim's stated rule.
fn colonizable_by(feed: &HudFeed, planet: EntityId) -> bool {
    let (Some(db), Some(p)) = (feed.content.as_deref(), feed.view.unit(planet)) else { return false };
    !p.flags.contains(UnitFlags::DYING)
        && p.hull[0] > sta_sim::Fixed::ZERO
        && (p.owner == sta_sim::world::NEUTRAL || p.owner == feed.player)
        && db.unit(p.def).is_some_and(|d| sta_sim::colony::is_colonizable_planet(db, d))
        && !feed.view.colonies.iter().any(|c| c.id == planet && c.colonizer.is_some())
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
    use bevy::ecs::system::RunSystemOnce;
    use sta_sim::view::{PlayerView, Relation, UnitView};
    use sta_sim::{Fixed, PlayerId, TeamId, WorldView};
    use std::sync::Arc;
    use crate::input::select::AimState;
    use crate::render::units::SelectRadius;
    use crate::sim_link::SimEntity;
    use crate::sim_link::CommandQueue;

    fn feed(selected: &[u32]) -> HudFeed {
        let db = sta_content::fake::db();
        let make = |id, odf, owner| UnitView {
            id: EntityId(id), def: db.key(odf).unwrap(), owner: PlayerId(owner),
            pos: SimVec3::ZERO, heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4], crew: None, energy: None, autonomy: None,
            flags: UnitFlags::SELECTABLE, progress: None,
        };
        HudFeed {
            view: Arc::new(WorldView { units: vec![make(1, "fx_cruiser", 1), make(2, "fx_miner", 1), make(3, "fx_yard", 1), make(4, "fx_cruiser", 2), make(5, "fx_moon", 255)], ..Default::default() }),
            player: PlayerId(1), selected: selected.iter().copied().map(EntityId).collect(), content: Some(Arc::new(db)), ..Default::default()
        }
    }

    fn player(id: u8, team: u8, relation: Option<Relation>) -> PlayerView {
        PlayerView { crew: None, id: PlayerId(id), team: TeamId(team), resources: Vec::new(), officers: [Fixed::ZERO; 2], eliminated: false,
            relations: relation.map(|r| vec![(PlayerId(if id == 1 { 2 } else { 1 }), r)]).unwrap_or_default() }
    }

    fn mine_available(f: &HudFeed) -> bool { target_mode_available(f, TargetMode::Mine) }

    #[test]
    fn roots_transport_attack_uses_only_ready_zero_weld_carriers() {
        let fake = sta_content::fake::db();
        let mut builder = sta_content::ContentDb::builder()
            .profile(sta_content::Profile::Fo4)
            .features(sta_content::Profile::Fo4.features());
        for weapon in fake.weapons() { builder = builder.weapon(weapon.clone()); }
        for unit in fake.units() {
            let mut unit = unit.clone();
            if unit.odf == "fx_cruiser" { unit.transport.welding_anim_speed = Some(Fixed::ZERO); }
            builder = builder.unit(unit);
        }
        let mut f = feed(&[1, 2]);
        f.content = Some(Arc::new(builder.build().unwrap()));
        let view = Arc::make_mut(&mut f.view);
        view.units[0].crew = Some([Fixed::from_int(40), Fixed::from_int(50)]);
        view.units[1].crew = Some([Fixed::from_int(40), Fixed::from_int(50)]);
        view.units[3].crew = Some([Fixed::from_int(30), Fixed::from_int(50)]);
        view.units[3].shields = [[Fixed::from_int(50), Fixed::from_int(50)]; 4];
        view.players = vec![player(1, 1, None), player(2, 2, None)];
        assert_eq!(armed_cursor_name(TargetMode::TransportAttack, true), "c_capture");
        assert_eq!(armed_cursor_name(TargetMode::TransportAttack, false), "c_nocapture");
        assert!(matches!(target_order(&f, TargetMode::TransportAttack, Some(EntityId(4)), None),
            Some((orders, ids)) if ids == vec![EntityId(1)]
                && matches!(orders.as_slice(), [Order::Transport { units, target: EntityId(4), attack: true }] if units == &[EntityId(1)])));
        assert!(matches!(target_order(&f, TargetMode::Transport, Some(EntityId(4)), None),
            Some((orders, _)) if matches!(orders.as_slice(), [Order::Transport { attack: false, .. }])));
        f.selected = vec![EntityId(2)];
        assert!(!target_mode_available(&f, TargetMode::TransportAttack));
        assert!(target_order(&f, TargetMode::TransportAttack, Some(EntityId(4)), None).is_none());
        f.selected = vec![EntityId(1)];
        assert!(target_order(&f, TargetMode::TransportAttack, Some(EntityId(1)), None).is_none());
    }

    #[test]
    fn repair_targets_a_repair_yard_or_lets_the_sim_choose() {
        let fake = sta_content::fake::db();
        let mut builder = sta_content::ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for weapon in fake.weapons() { builder = builder.weapon(weapon.clone()); }
        for unit in fake.units() {
            let mut unit = unit.clone();
            if unit.odf == "fx_yard" { unit.repair.repair_facility = Some(true); }
            builder = builder.unit(unit);
        }
        let mut f = feed(&[1, 3]);
        f.content = Some(Arc::new(builder.build().unwrap()));
        let p = Some(SimVec3::from_f32([10.0, 0.0, 20.0]));
        assert!(target_mode_available(&f, TargetMode::Repair), "a craft that can move is selected");
        // The yard under the cursor is named; only the craft of the mixed selection go.
        assert!(matches!(target_order(&f, TargetMode::Repair, Some(EntityId(3)), p), Some((orders, ids))
            if ids == vec![EntityId(1)] && matches!(orders.as_slice(), [Order::Repair { units, yard: Some(EntityId(3)) }] if units == &[EntityId(1)])));
        // Open space: the sim picks the best yard.
        assert!(matches!(target_order(&f, TargetMode::Repair, None, p), Some((orders, _)) if matches!(orders.as_slice(), [Order::Repair { yard: None, .. }])));
        // An enemy ship is no yard: the sim picks instead.
        assert!(matches!(target_order(&f, TargetMode::Repair, Some(EntityId(4)), p), Some((orders, _)) if matches!(orders.as_slice(), [Order::Repair { yard: None, .. }])));
        // A station alone cannot go to a yard.
        assert!(!target_mode_available(&feed(&[3]), TargetMode::Repair));
        assert_eq!(armed_cursor_name(TargetMode::Repair, true), "c_repair");
    }

    /// SIM-45: DECOMMISSION names a decommission yard under the cursor, else lets the sim choose.
    #[test]
    fn decommission_targets_a_yard_or_lets_the_sim_choose() {
        let fake = sta_content::fake::db();
        let mut builder = sta_content::ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for weapon in fake.weapons() { builder = builder.weapon(weapon.clone()); }
        for unit in fake.units() {
            let mut unit = unit.clone();
            if unit.odf == "fx_yard" { unit.repair.decommission_facility = Some(true); }
            builder = builder.unit(unit);
        }
        let mut f = feed(&[1]);
        f.content = Some(Arc::new(builder.build().unwrap()));
        let p = Some(SimVec3::from_f32([10.0, 0.0, 20.0]));
        assert!(target_mode_available(&f, TargetMode::Decommission));
        assert!(matches!(target_order(&f, TargetMode::Decommission, Some(EntityId(3)), p), Some((orders, ids))
            if ids == vec![EntityId(1)] && matches!(orders.as_slice(), [Order::Decommission { units, yard: Some(EntityId(3)) }] if units == &[EntityId(1)])));
        assert!(matches!(target_order(&f, TargetMode::Decommission, None, p), Some((orders, _)) if matches!(orders.as_slice(), [Order::Decommission { yard: None, .. }])));
        // An enemy's ship is no yard: the sim picks.
        assert!(matches!(target_order(&f, TargetMode::Decommission, Some(EntityId(4)), p), Some((orders, _)) if matches!(orders.as_slice(), [Order::Decommission { yard: None, .. }])));
        assert_eq!(armed_cursor_name(TargetMode::Decommission, true), "c_decommission");
    }

    /// SIM-41: a repair ship aimed at a ship that is not a yard is sent to repair it; aimed at a yard it
    /// goes there itself.
    #[test]
    fn repair_ships_aimed_at_a_ship_repair_it() {
        let fake = sta_content::fake::db();
        let mut builder = sta_content::ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for weapon in fake.weapons() { builder = builder.weapon(weapon.clone()); }
        for unit in fake.units() {
            let mut unit = unit.clone();
            if unit.odf == "fx_yard" { unit.repair.repair_facility = Some(true); }
            if unit.odf == "fx_cruiser" { unit.class = sta_content::ClassLabel::new("repairship"); }
            builder = builder.unit(unit);
        }
        let mut f = feed(&[1]);
        f.content = Some(Arc::new(builder.build().unwrap()));
        let p = Some(SimVec3::from_f32([10.0, 0.0, 20.0]));
        assert!(matches!(target_order(&f, TargetMode::Repair, Some(EntityId(4)), p), Some((orders, ids))
            if ids == vec![EntityId(1)] && matches!(orders.as_slice(), [Order::Repair { units, yard: Some(EntityId(4)) }] if units == &[EntityId(1)])));
        // At a yard it is a yard repair as before.
        assert!(matches!(target_order(&f, TargetMode::Repair, Some(EntityId(3)), p), Some((orders, _)) if matches!(orders.as_slice(), [Order::Repair { yard: Some(EntityId(3)), .. }])));
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
        // A mixed selection: Mine is offered and only the miner receives the order.
        let f = feed(&[1, 2]);
        assert!(mine_available(&f));
        assert!(matches!(target_order(&f, TargetMode::Mine, Some(EntityId(5)), p), Some((orders, ids)) if ids == [EntityId(2)] && matches!(orders.as_slice(), [Order::Mine { units, node: EntityId(5) }] if units == &[EntityId(2)])));
        // No miner among the selection: no order.
        assert!(target_order(&feed(&[1]), TargetMode::Mine, Some(EntityId(5)), p).is_none());
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

    #[test]
    fn armed_cursor_uses_the_click_order_and_respects_capture() {
        let mut world = World::new();
        world.insert_resource(feed(&[1]));
        world.insert_resource(HudState { targeting: Some(TargetMode::Attack), ..Default::default() });
        world.insert_resource(HudPointer(false));
        world.insert_resource(ghost::PlacementBounds::default());
        world.insert_resource(crate::ui::cursor::WantedCursor::default());
        let aim = AimState {
            cursor: Vec2::ZERO,
            ray: Ray3d::new(Vec3::new(0.0, 100.0, 0.0), Dir3::NEG_Y),
            clip_from_world: Mat4::IDENTITY,
            size: Vec2::new(800.0, 600.0),
        };
        world.insert_resource(Aim(Some(aim)));
        world.spawn((SimEntity(EntityId(4)), SelectRadius(10.0), GlobalTransform::default()));

        let check = |world: &mut World, expected| {
            world.resource_mut::<crate::ui::cursor::WantedCursor>().0 = "c_wormin";
            world.run_system_once(armed_cursor).unwrap();
            assert_eq!(world.resource::<crate::ui::cursor::WantedCursor>().0, expected);
        };
        check(&mut world, "c_attack");
        // The same click rule rejects a dying target.
        Arc::make_mut(&mut world.resource_mut::<HudFeed>().view).units[3].flags = UnitFlags::DYING;
        check(&mut world, "c_noattack");
        // HUD capture and missing aim force the normal pointer, even if Input ran with stale capture.
        world.resource_mut::<HudPointer>().0 = true;
        check(&mut world, crate::ui::cursor::DEFAULT);
        world.resource_mut::<HudPointer>().0 = false;
        world.resource_mut::<Aim>().0 = None;
        check(&mut world, crate::ui::cursor::DEFAULT);
    }

    #[test]
    fn armed_cursor_names_track_the_order_gate() {
        let p = Some(SimVec3::from_f32([10.0, 0.0, 20.0]));
        for (feed, mode, target, valid, refused) in [
            (feed(&[1]), TargetMode::Move, None, "c_move", "c_nomove"),
            (feed(&[1]), TargetMode::Attack, Some(EntityId(4)), "c_attack", "c_noattack"),
            (feed(&[2]), TargetMode::Mine, Some(EntityId(5)), "c_mining", "c_nomining"),
            (feed(&[3]), TargetMode::Rally, None, "c_rally", "c_nomove"),
        ] {
            assert_eq!(armed_cursor_name(mode, target_order(&feed, mode, target, p).is_some()), valid);
            assert_eq!(armed_cursor_name(mode, target_order(&feed, mode, Some(EntityId(99)), None).is_some()), refused);
        }
        let stale = feed(&[99]);
        assert_eq!(armed_cursor_name(TargetMode::Attack, target_order(&stale, TargetMode::Attack, Some(EntityId(4)), p).is_some()), "c_noattack");
    }

    fn click_app(feed: HudFeed, state: HudState, x: f32) -> App {
        let mut app = App::new();
        app.add_message::<OrderIssued>()
            .add_message::<Notice>()
            .insert_resource(feed)
            .insert_resource(state)
            .insert_resource(Aim(Some(AimState {
                cursor: Vec2::ZERO,
                ray: Ray3d::new(Vec3::new(x, 100.0, 0.0), Dir3::NEG_Y),
                clip_from_world: Mat4::IDENTITY,
                size: Vec2::new(800.0, 600.0),
            })))
            .insert_resource(HudPointer(false))
            .init_resource::<ghost::PlacementBounds>()
            .init_resource::<crate::ui::cursor::WantedCursor>()
            .init_resource::<CommandQueue>()
            .init_resource::<OrderTargets>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>();
        app
    }

    #[test]
    fn armed_attack_cursor_and_click_agree_on_valid_and_refused_targets() {
        let mut app = click_app(feed(&[1]), HudState { targeting: Some(TargetMode::Attack), ..Default::default() }, 0.0);
        app.world_mut().spawn((SimEntity(EntityId(4)), SelectRadius(10.0), GlobalTransform::default()));
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_attack");
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(matches!(app.world().resource::<CommandQueue>().0.as_slice(), [CommandKind::Order(Order::Attack { target: EntityId(4), .. })]));
        assert!(app.world().resource::<HudState>().targeting.is_none());

        app.world_mut().resource_mut::<HudState>().targeting = Some(TargetMode::Attack);
        app.world_mut().resource_mut::<CommandQueue>().0.clear();
        Arc::make_mut(&mut app.world_mut().resource_mut::<HudFeed>().view).units[3].flags = UnitFlags::DYING;
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_noattack");
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(app.world().resource::<CommandQueue>().0.is_empty());
        assert_eq!(app.world().resource::<HudState>().targeting, Some(TargetMode::Attack));

        Arc::make_mut(&mut app.world_mut().resource_mut::<HudFeed>().view).units[3].flags = UnitFlags::SELECTABLE;
        Arc::make_mut(&mut app.world_mut().resource_mut::<HudFeed>().view).visibility = vec![(PlayerId(1), vec![EntityId(1)])];
        app.insert_resource(crate::render::units::LocalPlayer(Some(PlayerId(1))));
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_noattack");
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(app.world().resource::<CommandQueue>().0.is_empty());

        // A stale selection is cleared by the existing click path before presentation.
        app.world_mut().resource_mut::<HudFeed>().selected = vec![EntityId(99)];
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(app.world().resource::<HudState>().targeting.is_none());
        app.world_mut().resource_mut::<HudState>().targeting = Some(TargetMode::Attack);
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Escape);
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(app.world().resource::<HudState>().targeting.is_none());
    }

    #[test]
    fn armed_attack_reaches_seen_target_behind_unseen_resource_and_refuses_seen_resource() {
        let mut f = feed(&[1]);
        Arc::make_mut(&mut f.view).visibility = vec![(PlayerId(1), vec![EntityId(1), EntityId(4)])];
        let mut app = click_app(f, HudState { targeting: Some(TargetMode::Attack), ..Default::default() }, 0.0);
        app.insert_resource(crate::render::units::LocalPlayer(Some(PlayerId(1))));
        app.world_mut().spawn((SimEntity(EntityId(4)), SelectRadius(10.0), GlobalTransform::default()));
        app.world_mut().spawn((SimEntity(EntityId(5)), SelectRadius(10.0), GlobalTransform::from(Transform::from_xyz(0.0, 50.0, 0.0))));
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_attack");
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(matches!(app.world().resource::<CommandQueue>().0.as_slice(), [CommandKind::Order(Order::Attack { target: EntityId(4), .. })]));

        app.world_mut().resource_mut::<CommandQueue>().0.clear();
        app.world_mut().resource_mut::<HudState>().targeting = Some(TargetMode::Attack);
        Arc::make_mut(&mut app.world_mut().resource_mut::<HudFeed>().view).visibility[0].1.push(EntityId(5));
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_noattack");
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(app.world().resource::<CommandQueue>().0.is_empty());
    }

    #[test]
    fn armed_build_cursor_and_click_agree_on_site_and_cost() {
        let mut f = feed(&[1]);
        let fake = f.content.as_ref().unwrap();
        let mut builder = sta_content::ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for weapon in fake.weapons() { builder = builder.weapon(weapon.clone()); }
        for unit in fake.units() {
            let mut unit = unit.clone();
            if unit.odf == "fx_cruiser" { unit.build.list = vec![sta_content::DefRef::new("fx_station")]; }
            builder = builder.unit(unit);
        }
        let db = builder.build().unwrap();
        assert_eq!(f.view.unit(EntityId(1)).unwrap().def, db.key("fx_cruiser").unwrap());
        f.content = Some(Arc::new(db));
        Arc::make_mut(&mut f.view).players = vec![player(1, 1, None)];
        Arc::make_mut(&mut f.view).players[0].resources = vec![Fixed::from_int(3000)];
        let what = f.content.as_ref().unwrap().key("fx_station").unwrap();
        let armed = Armed { builder: EntityId(1), what, odf: "fx_station".into() };
        let mut app = click_app(f, HudState { placing: Some(armed.clone()), ..Default::default() }, 1000.0);
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_build");

        app.world_mut().resource_mut::<Aim>().0.as_mut().unwrap().ray = Ray3d::new(Vec3::new(0.0, 100.0, 0.0), Dir3::NEG_Y);
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_nobuild");
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(app.world().resource::<CommandQueue>().0.is_empty());
        assert!(app.world().resource::<HudState>().placing.is_some());

        app.world_mut().resource_mut::<Aim>().0.as_mut().unwrap().ray = Ray3d::new(Vec3::new(1000.0, 100.0, 0.0), Dir3::NEG_Y);
        let lo = SimVec3::from_f32([-500.0, -500.0, -500.0]);
        let hi = SimVec3::from_f32([500.0, 500.0, 500.0]);
        app.world_mut().resource_mut::<ghost::PlacementBounds>().0 = Some((lo, hi));
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_nobuild");
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(app.world().resource::<CommandQueue>().0.is_empty());
        app.world_mut().resource_mut::<ghost::PlacementBounds>().0 = None;
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_build");
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(matches!(app.world().resource::<CommandQueue>().0.as_slice(), [CommandKind::Order(Order::Build { builder: EntityId(1), what: w, .. })] if *w == what));
        assert!(app.world().resource::<HudState>().placing.is_none());

        app.world_mut().resource_mut::<HudState>().placing = Some(armed);
        app.world_mut().resource_mut::<CommandQueue>().0.clear();
        Arc::make_mut(&mut app.world_mut().resource_mut::<HudFeed>().view).players[0].resources[0] = Fixed::ZERO;
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_nobuild");
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(app.world().resource::<CommandQueue>().0.is_empty());
        assert!(app.world().resource::<HudState>().placing.is_some());
    }

    #[test]
    fn armed_cursor_update_obeys_input_capture_guards() {
        let mut app = click_app(feed(&[1]), HudState { targeting: Some(TargetMode::Move), ..Default::default() }, 1000.0);
        app.add_systems(Update, armed_cursor
            .run_if(not(crate::ui::presentation::cutscene_running))
            .run_if(crate::ui::ingame_menu::menu_closed)
            .run_if(crate::cheats::input::game_input_free));
        let name = |app: &App| app.world().resource::<crate::ui::cursor::WantedCursor>().0;
        app.update();
        assert_eq!(name(&app), "c_move");
        let mut menu = crate::ui::ingame_menu::InGameMenu::default();
        menu.open = true;
        app.insert_resource(menu);
        app.world_mut().resource_mut::<crate::ui::cursor::WantedCursor>().0 = crate::ui::cursor::DEFAULT;
        app.update();
        assert_eq!(name(&app), crate::ui::cursor::DEFAULT);
        app.world_mut().resource_mut::<crate::ui::ingame_menu::InGameMenu>().open = false;
        let mut cheat = crate::cheats::input::CheatInput::default();
        cheat.open = true;
        app.insert_resource(cheat);
        app.update();
        assert_eq!(name(&app), crate::ui::cursor::DEFAULT);
        app.world_mut().resource_mut::<crate::cheats::input::CheatInput>().open = false;
        let mut cine = crate::ui::presentation::CineractiveState::default();
        cine.on = true;
        app.insert_resource(cine);
        app.update();
        assert_eq!(name(&app), crate::ui::cursor::DEFAULT);
        app.world_mut().resource_mut::<crate::ui::presentation::CineractiveState>().on = false;
        app.update();
        assert_eq!(name(&app), "c_move");
    }

    #[test]
    fn transport_cursor_and_click_share_the_ready_carrier_and_target_gates() {
        let fake = sta_content::fake::db();
        let mut builder = sta_content::ContentDb::builder()
            .profile(sta_content::Profile::Sta2Classic)
            .features(sta_content::Profile::Sta2Classic.features());
        for weapon in fake.weapons() { builder = builder.weapon(weapon.clone()); }
        for unit in fake.units() { builder = builder.unit(unit.clone()); }
        let db = Arc::new(builder.build().unwrap());
        let mut f = feed(&[1]);
        f.content = Some(db);
        let view = Arc::make_mut(&mut f.view);
        view.units[0].crew = Some([Fixed::from_int(40), Fixed::from_int(50)]);
        view.units[3].crew = Some([Fixed::from_int(30), Fixed::from_int(50)]);
        view.players = vec![player(1, 1, None), player(2, 2, None)];
        let mut app = click_app(f, HudState { targeting: Some(TargetMode::Transport), ..Default::default() }, 0.0);
        app.world_mut().spawn((SimEntity(EntityId(4)), SelectRadius(10.0), GlobalTransform::default()));
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_transport");
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(matches!(app.world().resource::<CommandQueue>().0.as_slice(), [CommandKind::Order(Order::Transport { target: EntityId(4), .. })]));

        app.world_mut().resource_mut::<HudState>().targeting = Some(TargetMode::Transport);
        app.world_mut().resource_mut::<CommandQueue>().0.clear();
        Arc::make_mut(&mut app.world_mut().resource_mut::<HudFeed>().view).units[3].crew = None;
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_notransport");
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(app.world().resource::<CommandQueue>().0.is_empty());
    }

    #[test]
    fn colonize_cursor_and_click_share_the_planet_gate() {
        use sta_content::{ColonizePodDef, ContentDb, DefRef, Hardpoint, PlanetRole, PopulationRules, Rules, UnitDef, WeaponDef, WeaponKind};
        let mut pod = WeaponDef::new("ui_pod", "colonizerpod", WeaponKind::Projectile);
        pod.colonize = Some(ColonizePodDef { party_size: Fixed::from_int(100), orbit_range: Fixed::from_int(150) });
        let mut ship = UnitDef::new("ui_colony", "colony");
        ship.movement.max_speed = Fixed::ONE;
        ship.hardpoints = vec![Hardpoint { weapon: DefRef::new("ui_pod"), node: "hp01".into(), arc: None }];
        ship.raw = Arc::new(sta_odf::resolve("s", &mut |_| Some("colony_ship = 1".into())).unwrap());
        let mut planet = UnitDef::new("ui_planet", "planet");
        planet.planet_role = Some(PlanetRole::Producer);
        planet.planet_colonizable = true;
        planet.planet_max_population = 1;
        let rules = Rules::default().with_population(PopulationRules {
            levels: [Fixed::from_int(1000), Fixed::from_int(2000), Fixed::from_int(3000), Fixed::from_int(5000)],
            civilian_rate_modifier: Fixed::from_f64(0.2),
        });
        let db = Arc::new(ContentDb::builder().weapon(pod).unit(ship).unit(planet).build().unwrap().with_rules(rules));
        let unit = |id, odf, owner| UnitView {
            id: EntityId(id), def: db.key(odf).unwrap(), owner, pos: SimVec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO, hull: [Fixed::ONE; 2],
            shields: [[Fixed::ZERO; 2]; 4], crew: None, energy: None, autonomy: None,
            flags: UnitFlags::SELECTABLE, progress: None,
        };
        let f = HudFeed {
            view: Arc::new(WorldView { units: vec![unit(1, "ui_colony", PlayerId(1)), unit(2, "ui_planet", sta_sim::world::NEUTRAL)], ..Default::default() }),
            player: PlayerId(1), selected: vec![EntityId(1)], content: Some(db), ..Default::default()
        };
        let mut app = click_app(f, HudState { targeting: Some(TargetMode::Colonize), ..Default::default() }, 0.0);
        app.world_mut().spawn((SimEntity(EntityId(2)), SelectRadius(10.0), GlobalTransform::default()));
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_colonize");
        app.world_mut().resource_mut::<ButtonInput<MouseButton>>().press(MouseButton::Left);
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(matches!(app.world().resource::<CommandQueue>().0.as_slice(), [CommandKind::Order(Order::Colonize { planet: EntityId(2), .. })]));

        app.world_mut().resource_mut::<HudState>().targeting = Some(TargetMode::Colonize);
        app.world_mut().resource_mut::<CommandQueue>().0.clear();
        Arc::make_mut(&mut app.world_mut().resource_mut::<HudFeed>().view).units[1].owner = PlayerId(2);
        app.world_mut().run_system_once(armed_cursor).unwrap();
        assert_eq!(app.world().resource::<crate::ui::cursor::WantedCursor>().0, "c_nocolonize");
        app.world_mut().run_system_once(place_on_click).unwrap();
        assert!(app.world().resource::<CommandQueue>().0.is_empty());
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod colonize_tests;
#[cfg(test)]
mod transport_tests;
#[cfg(test)]
mod recrew_tests;
#[cfg(test)]
mod pd_tests;
