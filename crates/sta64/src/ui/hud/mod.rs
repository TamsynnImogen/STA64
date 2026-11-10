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
//! - **Grid**: 5 x 3 cells. The top page issues UI-02's orders on the own
//!   selection (Stop, the three formations) and opens the build page for a
//!   builder. The build page lists the builder's build list, through the tech
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
pub mod layout;
pub mod model;
mod panels;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use sta_content::GuiLayout;
use sta_sim::command::{CommandKind, Order};
use sta_sim::{ContentKey, EntityId, Vec3 as SimVec3};

pub use layout::{HudLayout, InfoHeight};
pub use panels::HudRoot;
pub use model::{Act, HudFeed, Page};
use model::{Models, command, models};
use panels::{GridCell, GridPanel, GroupTile, InfoPanel, ResourceStrip};

use super::notices::{Notice, NoticePlugin, NoticeRoot, Severity};
use super::radar::{RadarPlugin, RadarRoot};
use super::theme::{ButtonClicked, Theme, ThemePlugin};
use crate::app::GameSet;
use crate::input::orders::Aim;
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
    !pointer.is_some_and(|p| p.0) && !state.is_some_and(|s| s.placing.is_some())
}

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ThemePlugin>() {
            app.add_plugins(ThemePlugin);
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
            .init_resource::<CommandQueue>()
            .add_message::<OrderIssued>()
            .add_message::<SelectionChanged>()
            .init_resource::<Selection>()
            .add_systems(Startup, panels::spawn_root.after(crate::ui::radar::spawn_radar))
            .add_systems(
                Update,
                (
                    feed_from_link.run_if(resource_exists::<SimLink>).run_if(not(resource_exists::<ExternalFeed>)),
                    fit_screen,
                    refresh,
                    host,
                    pointer,
                    press,
                    place_on_click.run_if(resource_exists::<Aim>),
                )
                    .chain()
                    .in_set(GameSet::Ui),
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
    if state.page == Page::Build && model::builder(&feed).is_none() {
        state.page = Page::Top;
    }
    let now = models(&feed, state.page, layout.0.no_resource);
    let height = now.info.height();
    let layout_changed = layout.is_changed() || state.height != height || state.drawn.is_none();
    let drawn = state.drawn.take();
    let l = &layout.0;
    let row = l.bottom_row(height);

    if let Ok((e, mut node, kids)) = strip.single_mut() {
        if layout_changed {
            panels::place(&mut node, l.resources);
        }
        if layout_changed || drawn.as_ref().is_none_or(|d| d.resources != now.resources) {
            clear(&mut commands, kids);
            panels::fill_resources(&mut commands, &theme, e, &now.resources);
        }
    }
    if let Ok((e, mut node, kids)) = info_panel.single_mut() {
        if layout_changed {
            panels::place(&mut node, row.info);
        }
        if layout_changed || drawn.as_ref().is_none_or(|d| d.info != now.info) {
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
        m.resources.iter().map(|e| format!("{}={}", e.name, e.text)).collect::<Vec<_>>().join(" "),
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
) {
    for ButtonClicked(e) in clicks.read() {
        if let Ok(GroupTile(id)) = tiles.get(*e) {
            let shift = keys.as_ref().is_some_and(|k| k.pressed(KeyCode::ShiftLeft) || k.pressed(KeyCode::ShiftRight));
            selection.0 = if shift { selection.0.iter().copied().filter(|s| s != id).collect() } else { vec![*id] };
            changed.write(SelectionChanged);
            continue;
        }
        let Ok(GridCell(act)) = cells.get(*e) else { continue };
        match act {
            Act::OpenBuild => state.page = Page::Build,
            Act::Back => {
                state.page = Page::Top;
                state.placing = None;
            }
            Act::Build { builder, what, odf, site: true } => {
                state.placing = Some(Armed { builder: *builder, what: *what, odf: odf.clone() });
                notices.write(Notice::new(Severity::Info, format!("Place {odf}: click the map (Esc cancels)")));
            }
            other => {
                if let Some((kind, units)) = command(&feed, other) {
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
) {
    let Some(armed) = state.placing.clone() else { return };
    if keys.is_some_and(|k| k.just_pressed(KeyCode::Escape)) || feed.view.unit(armed.builder).is_none() {
        state.placing = None;
        return;
    }
    let Some(buttons) = buttons else { return };
    if !buttons.just_pressed(MouseButton::Left) || pointer.0 {
        return;
    }
    let (Some(a), Some(b)) = (aim.0, feed.view.unit(armed.builder)) else { return };
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
    queue.0.push(CommandKind::Order(Order::Build { builder: armed.builder, what: armed.what, at: Some(at) }));
    issued.write(OrderIssued { units: vec![armed.builder], kind: OrderKind::Build });
    state.placing = None;
}

#[cfg(test)]
mod tests;
