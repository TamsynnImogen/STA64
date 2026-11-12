//! The minimap / radar (UI-05), after Refit's `ui/HUD.md`, "The minimap".
//!
//! A square of [`RADAR_SIDE`] units at the bottom left, in an LCARS panel of the
//! theme (UI-03), with the two filter strips above it. What it draws, in
//! Refit's order: the dark backing; one blip per unit the player's side sees
//! (ships round, stations square, never under 7 / 10 units; a white ring round
//! a selected unit; moons and planets as dim discs); and the camera's outline.
//! Refit's fog masks, pings and the map's picture need seam data the views do
//! not carry yet (see the notes on [`RadarFeed`]).
//!
//! - Colours: the sim has no per-team map colour, so a blip is coloured by the
//!   player's relation to its owner, Refit's fallback: own `good`, ally
//!   `accent2`, enemy `bad`, neutral `warn`.
//! - Filters: ships and stations each cycle all / allies / enemies / neutral
//!   (allies include the player's own). Click a dot to pick a state; click the
//!   name to go to the next, as stock.
//! - Clicks: the square sends a [`RadarClick`] with the map point under the
//!   pointer and the button; the HUD / input turn that into a camera move or a
//!   `GO_WARP` order (UI-02, INT-M3).
//!
//! Positions are the sim's map space: x right, z up the screen (Refit's "v
//! running against z").

// Wired into the game by INT-M3; until then only the frame runs use it.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::picking::Pickable;
use bevy::picking::events::PointerClick;
use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use sta_content::ContentDb;
use sta_sim::view::{Relation, UnitFlags};
use sta_sim::{EntityId, PlayerId, WorldView};

use super::theme::{Theme, ThemePlugin, mix, panel};
use super::gui_sprites::GuiImage;
use crate::app::GameSet;
use crate::sim_link::{ContentRes, Selection, SimLink};
use crate::ui::i18n::tr;
use crate::ui::profile::ProfileSync;
use crate::ui::profile::UiProfile;
use crate::ui::hud::HudConfig;
use crate::ui::hud::stock::layout::StockLayout;

/// The square's side in theme units (Refit `MinimapSize`, at most 45% of the
/// 1080-unit screen).
pub const RADAR_SIDE: f32 = 300.0;
/// Smallest blip edge, in units: ships, stations, moons and planets.
pub const SHIP_DOT: f32 = 7.0;
pub const STATION_DOT: f32 = 10.0;
pub const BODY_DOT: f32 = 14.0;
/// Edge of a filter dot, in units.
const FILTER_DOT: f32 = 14.0;

/// What a blip stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ship,
    Station,
    /// A moon or planet (or a resource node): drawn dim, never filtered.
    Body,
}

/// How the player stands toward a unit's owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    Own,
    Ally,
    Neutral,
    Enemy,
}

/// A filter state; ships and stations each have one (Refit: 0 all, 1 allies,
/// 2 enemies, 3 neutral).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Allies,
    Enemies,
    Neutral,
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::All, Filter::Allies, Filter::Enemies, Filter::Neutral];

    pub fn next(self) -> Filter {
        match self {
            Filter::All => Filter::Allies,
            Filter::Allies => Filter::Enemies,
            Filter::Enemies => Filter::Neutral,
            Filter::Neutral => Filter::All,
        }
    }

    /// The tooltip, as stock names the states.
    pub fn name(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Allies => "Allies",
            Filter::Enemies => "Enemies",
            Filter::Neutral => "Neutral",
        }
    }

    pub fn passes(self, s: Standing) -> bool {
        match self {
            Filter::All => true,
            Filter::Allies => matches!(s, Standing::Own | Standing::Ally),
            Filter::Enemies => s == Standing::Enemy,
            Filter::Neutral => s == Standing::Neutral,
        }
    }
}

/// The map's box in the square: a square `size` units across from `min`
/// (x, z).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadarMap {
    pub min: Vec2,
    pub size: f32,
}

impl RadarMap {
    /// A square round every unit of `view`, 10% larger each way, `None` for an
    /// empty world.
    pub fn fit(view: &WorldView) -> Option<RadarMap> {
        let mut pts = view.units.iter().map(|u| Vec2::new(u.pos.x.to_f32(), u.pos.z.to_f32()));
        let first = pts.next()?;
        let (lo, hi) = pts.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p)));
        let size = ((hi - lo).max_element() * 1.2).max(500.0);
        Some(RadarMap { min: (lo + hi) / 2.0 - Vec2::splat(size / 2.0), size })
    }

    /// The square round a map's rectangle `lo..hi` (x, z): the longer side fills it and the
    /// shorter is centred, so a map that is not square keeps its aspect.
    pub fn of_bounds(lo: Vec2, hi: Vec2) -> RadarMap {
        let size = (hi - lo).max_element().max(1.0);
        RadarMap { min: (lo + hi) / 2.0 - Vec2::splat(size / 2.0), size }
    }

    /// Grow to hold `other` too, never shrink: the picture does not jump as
    /// units come and go.
    pub fn grow(self, other: RadarMap) -> RadarMap {
        let lo = self.min.min(other.min);
        let hi = (self.min + Vec2::splat(self.size)).max(other.min + Vec2::splat(other.size));
        RadarMap { min: lo, size: (hi - lo).max_element() }
    }

    /// Map point (x, z) to the square: `(u, v)` in `0..=1`, `v` down the screen.
    pub fn to_square(self, p: Vec2) -> Vec2 {
        let n = (p - self.min) / self.size;
        Vec2::new(n.x, 1.0 - n.y)
    }

    /// The map point (x, z) under `(u, v)` of the square.
    pub fn to_world(self, uv: Vec2) -> Vec2 {
        self.min + Vec2::new(uv.x, 1.0 - uv.y) * self.size
    }
}

/// What the radar draws from: set by [`feed_from_link`] in the game, by the
/// bench in frame runs, and by tests.
///
/// Not carried by the views yet, so not drawn: fog and shroud masks (needs
/// the scan grids, SIM-12), pings, last-known enemies, per-team map colours.
#[derive(Resource, Clone)]
pub struct RadarFeed {
    pub view: Arc<WorldView>,
    /// Whose radar this is.
    pub player: PlayerId,
    pub selected: Vec<EntityId>,
    /// Tells stations and moons from ships; without it every unit is a ship.
    pub content: Option<Arc<ContentDb>>,
    /// The camera's view on the ground: the corners of its footprint in map space (x, z), in
    /// order round it (a trapezoid under a tilted camera).
    pub camera: Option<[Vec2; 4]>,
}

impl Default for RadarFeed {
    fn default() -> Self {
        RadarFeed { view: Arc::default(), player: PlayerId(0), selected: Vec::new(), content: None, camera: None }
    }
}

/// The radar's own state: the filters and the map's box.
#[derive(Resource, Debug, Default, Clone)]
pub struct RadarState {
    pub ships: Filter,
    pub stations: Filter,
    /// `None` until the first view; then it only grows (see [`RadarMap::grow`])
    /// unless a source sets the map's real box.
    pub map: Option<RadarMap>,
    /// Set the map once and keep it: the real map's box from the game setup.
    pub map_fixed: bool,
}

/// The map point the player clicked on the square.
#[derive(Message, Debug, Clone, Copy, PartialEq)]
pub struct RadarClick {
    /// Map space (x, z).
    pub at: Vec2,
    pub button: PointerButton,
}

/// One blip, in the square's `(u, v)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blip {
    pub id: EntityId,
    pub kind: Kind,
    pub standing: Standing,
    pub at: Vec2,
    pub selected: bool,
}

impl Blip {
    /// The blip's edge in units.
    pub fn size(&self) -> f32 {
        match self.kind {
            Kind::Ship => SHIP_DOT,
            Kind::Station => STATION_DOT,
            Kind::Body => BODY_DOT,
        }
    }
}

fn standing(view: &WorldView, player: PlayerId, owner: PlayerId) -> Standing {
    if owner == player {
        return Standing::Own;
    }
    let Some(me) = view.player(player) else { return Standing::Neutral };
    if let Some((_, r)) = me.relations.iter().find(|(p, _)| *p == owner) {
        return match r {
            Relation::Ally => Standing::Ally,
            Relation::Enemy => Standing::Enemy,
            _ => Standing::Neutral,
        };
    }
    // No relation recorded (the bench's scenes): same team allies, another
    // team's units are enemies, ownerless ones neutral.
    match view.player(owner) {
        Some(o) if o.team == me.team => Standing::Ally,
        Some(_) => Standing::Enemy,
        None => Standing::Neutral,
    }
}

fn kind(content: Option<&ContentDb>, def: sta_sim::ContentKey) -> Kind {
    let Some(d) = content.and_then(|c| c.unit(def)) else { return Kind::Ship };
    let class = d.class.0.as_str();
    if d.resource.is_some() || class.contains("planet") || class.contains("moon") {
        Kind::Body
    } else if d.movement.max_speed.to_f32() <= 0.0 {
        Kind::Station
    } else {
        Kind::Ship
    }
}

/// The blips the player's radar shows: what its side sees (`visible_to`), minus
/// what the filters hide, and not what is dying, in id order.
pub fn blips(feed: &RadarFeed, state: &RadarState, map: &RadarMap) -> Vec<Blip> {
    let db = feed.content.as_deref();
    feed.view
        .visible_to(feed.player)
        .into_iter()
        .filter(|u| !u.flags.contains(UnitFlags::DYING))
        .filter_map(|u| {
            let kind = kind(db, u.def);
            let st = standing(&feed.view, feed.player, u.owner);
            let filter = match kind {
                Kind::Ship => state.ships,
                Kind::Station => state.stations,
                Kind::Body => Filter::All,
            };
            filter.passes(st).then(|| Blip {
                id: u.id,
                kind,
                standing: st,
                at: map.to_square(Vec2::new(u.pos.x.to_f32(), u.pos.z.to_f32())),
                selected: feed.selected.contains(&u.id),
            })
        })
        .collect()
}

/// A blip's colour: Refit's relation fallback, dim for bodies.
pub fn blip_colour(theme: &Theme, b: &Blip) -> Color {
    match (b.kind, b.standing) {
        (Kind::Body, _) => theme.text_off,
        (_, Standing::Own) => theme.good,
        (_, Standing::Ally) => theme.accent2,
        (_, Standing::Enemy) => theme.bad,
        (_, Standing::Neutral) => theme.warn,
    }
}

/// A filter dot's colour: the state's blip colour, dim unless it is the one in force.
pub fn dot_colour(theme: &Theme, f: Filter, lit: bool) -> Color {
    let c = match f {
        Filter::All => theme.text,
        Filter::Allies => theme.accent2,
        Filter::Enemies => theme.bad,
        Filter::Neutral => theme.warn,
    };
    if lit { c } else { mix(c, theme.panel, 0.65) }
}

// ---- the widgets ----------------------------------------------------------

/// The radar's outermost node.
#[derive(Component)]
pub struct RadarRoot;

/// The replaceable presentation below the persistent radar owner.
#[derive(Component)]
struct RadarChrome;

/// The map square; blips and the camera outline are its children.
#[derive(Component)]
pub struct RadarSquare;

#[derive(Component)]
pub struct RadarBlip(pub EntityId);

/// One side of the camera's outline in the square (0 to 3).
#[derive(Component)]
pub struct RadarCamera(pub usize);

/// A dot of a filter strip: pick `state` for ships or stations.
#[derive(Component, Debug, Clone, Copy)]
pub struct FilterDot {
    pub station: bool,
    pub state: Filter,
}

/// The name beside a strip: click for the next state.
#[derive(Component, Debug, Clone, Copy)]
pub struct FilterName {
    pub station: bool,
}

/// Blip entities by sim entity.
#[derive(Resource, Default)]
pub struct BlipEntities(BTreeMap<EntityId, Entity>);

fn text(s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(s.into()),
        TextFont { font_size: bevy::text::FontSize::Px(size), ..default() },
        TextColor(color),
        Pickable::IGNORE,
    )
}

fn strip(theme: &Theme, station: bool) -> impl Bundle {
    let dots: Vec<_> = Filter::ALL
        .iter()
        .map(|&state| {
            let ring = state == Filter::All;
            (
                FilterDot { station, state },
                Node {
                    width: Val::Px(FILTER_DOT),
                    height: Val::Px(FILTER_DOT),
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: if station { BorderRadius::px(2.0, 2.0, 2.0, 2.0) } else { BorderRadius::MAX },
                    ..default()
                },
                BackgroundColor(if ring { Color::NONE } else { dot_colour(theme, state, false) }),
                BorderColor::all(dot_colour(theme, state, false)),
            )
        })
        .collect();
    (
        Node { align_items: AlignItems::Center, column_gap: Val::Px(theme.gap * 0.75), ..default() },
        Children::spawn((
            Spawn((
                FilterName { station },
                Node { margin: UiRect::right(Val::Px(2.0)), ..default() },
                children![text(tr(if station { "Stations" } else { "Ships" }), theme.micro_size, theme.text_dim)],
            )),
            SpawnIter(dots.into_iter()),
        )),
    )
}

fn stock_rect(r: crate::ui::hud::layout::R) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(r.x),
        top: Val::Px(r.y),
        width: Val::Px(r.w),
        height: Val::Px(r.h),
        ..default()
    }
}

fn spawn_stock(commands: &mut Commands, root: Entity, theme: &Theme, cfg: &HudConfig) {
    let Some(gui) = cfg.gui.as_ref() else { return };
    let layout = StockLayout::new(gui, Vec2::new(1080.0 * cfg.aspect, 1080.0));
    let area = layout.panel("minimap");
    let chrome = commands.spawn((
        RadarChrome,
        Node {
            position_type: PositionType::Relative,
            width: Val::Px(area.map_or(0.0, |r| r.w)),
            height: Val::Px(area.map_or(0.0, |r| r.h)),
            display: if area.is_some() { Display::Flex } else { Display::None },
            ..default()
        },
    )).id();
    commands.entity(root).add_child(chrome);
    let Some(_) = area else { return };

    for piece in layout.pieces("minimapBackgroundPanel") {
        commands.spawn((
            stock_rect(piece.rect),
            GuiImage::new(piece.name).with_frame(piece.frame),
            Pickable::IGNORE,
            ChildOf(chrome),
        ));
    }

    let Some(display) = layout.rect("minimapDisplayArea") else { return };
    commands.spawn((
        stock_rect(display),
        BackgroundColor(mix(theme.panel, theme.track, 0.5)),
        Pickable::IGNORE,
        ChildOf(chrome),
    ));
    if let Some(sprite) = layout.name("minimapBackgroundSprite") {
        commands.spawn((stock_rect(display), GuiImage::new(sprite), Pickable::IGNORE, ChildOf(chrome)));
    }
    commands.spawn((
        RadarSquare,
        RelativeCursorPosition::default(),
        Node { overflow: Overflow::clip(), ..stock_rect(display) },
        BackgroundColor(Color::NONE),
        ChildOf(chrome),
    ));

    for (station, key, fallback) in [
        (false, "minimapShipFilterArea", "mapfilter_ship_button"),
        (true, "minimapStationFilterArea", "mapfilter_station_button"),
    ] {
        let Some(rect) = layout.rect(key) else { continue };
        let picture_key = if station { "minimapStationFilterButton" } else { "minimapShipFilterButton" };
        let picture = layout.name(picture_key).unwrap_or(fallback);
        let filter = commands.spawn((
            FilterName { station },
            crate::ui::notices::Tooltip::new(tr(if station { "Stations" } else { "Ships" })),
            stock_rect(rect),
            GuiImage::new(picture),
            ChildOf(chrome),
        )).id();
        // Tiny authored filter icons (FO) have no room for an extra caption;
        // their tooltip retains the label without overlapping the neighbouring icon.
        if rect.h >= 18.0 {
            commands.entity(filter).with_child(text(
                tr(if station { "Stations" } else { "Ships" }), theme.micro_size.max(18.0), theme.text,
            ));
        }
    }
}

/// Spawn the radar bottom left, in the theme's panel. Does nothing if one exists.
pub fn spawn_radar(
    mut commands: Commands,
    theme: Res<Theme>,
    profile: Option<Res<UiProfile>>,
    config: Option<Res<HudConfig>>,
    existing: Query<(Entity, Option<&Children>), With<RadarRoot>>,
) {
    let t = &*theme;
    let stock = profile.is_some_and(|p| p.is_stock()) && config.as_ref().is_some_and(|c| c.gui.is_some());
    let root = if let Some((root, children)) = existing.iter().next() {
        if children.is_some_and(|children| !children.is_empty()) { return; }
        root
    } else {
        commands.spawn((
            RadarRoot,
            Node { position_type: PositionType::Absolute, left: Val::Px(10.0), bottom: Val::Px(10.0), ..default() },
        )).id()
    };
    if stock {
        spawn_stock(&mut commands, root, t, config.as_ref().unwrap());
        return;
    }
    let content = (
        Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(t.gap * 0.5), ..default() },
        children![
            (
                Node { justify_content: JustifyContent::SpaceBetween, width: Val::Px(RADAR_SIDE), ..default() },
                children![strip(t, false), strip(t, true)],
            ),
            (
                RadarSquare,
                RelativeCursorPosition::default(),
                Node {
                    width: Val::Px(RADAR_SIDE),
                    height: Val::Px(RADAR_SIDE),
                    overflow: Overflow::clip(),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(mix(t.panel, t.track, 0.5)),
                BorderColor::all(t.accent2),
            ),
        ],
    );
    commands.entity(root).with_child((RadarChrome, panel(t, &tr("Radar"), content)));
}

// ---- systems ---------------------------------------------------------------

/// Copy the game's view, player, selection and content into the feed.
pub fn feed_from_link(
    link: Res<SimLink>,
    selection: Res<Selection>,
    content: Option<Res<ContentRes>>,
    mut feed: ResMut<RadarFeed>,
) {
    feed.view = link.curr().clone();
    feed.player = link.player();
    feed.selected.clone_from(&selection.0);
    feed.content = content.map(|c| c.0.clone());
}

/// Place the blips and the camera outline.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
pub fn refresh(
    mut commands: Commands,
    theme: Res<Theme>,
    feed: Res<RadarFeed>,
    mut state: ResMut<RadarState>,
    mut ents: ResMut<BlipEntities>,
    square: Query<(Entity, &Node), (With<RadarSquare>, Without<RadarCamera>)>,
    mut nodes: Query<(&mut Node, &mut BackgroundColor, &mut BorderColor), (Without<RadarCamera>, Without<RadarSquare>)>,
    outline: Query<(Entity, &RadarCamera)>,
    mut lines_q: Query<(&mut Node, &mut UiTransform), (With<RadarCamera>, Without<RadarSquare>)>,
) {
    let Ok((square, square_node)) = square.single() else { return };
    if !state.map_fixed
        && let Some(fit) = RadarMap::fit(&feed.view)
    {
        state.map = Some(state.map.map_or(fit, |m| m.grow(fit)));
    }
    let Some(map) = state.map else { return };
    let list = blips(&feed, &state, &map);
    let live: Vec<EntityId> = list.iter().map(|b| b.id).collect();
    ents.0.retain(|id, e| {
        let keep = live.contains(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    let t = &*theme;
    for b in &list {
        let d = b.size();
        let node = Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(b.at.x * 100.0),
            top: Val::Percent(b.at.y * 100.0),
            margin: UiRect::all(Val::Px(-d / 2.0)),
            width: Val::Px(d),
            height: Val::Px(d),
            border: UiRect::all(Val::Px(if b.selected { 2.0 } else { 0.0 })),
            border_radius: if b.kind == Kind::Station { BorderRadius::ZERO } else { BorderRadius::MAX },
            ..default()
        };
        let bg = blip_colour(t, b);
        let ring = if b.selected { Color::WHITE } else { Color::NONE };
        match ents.0.get(&b.id).and_then(|&e| nodes.get_mut(e).ok()) {
            Some((mut n, mut c, mut br)) => {
                if *n != node {
                    *n = node;
                }
                if c.0 != bg {
                    c.0 = bg;
                }
                *br = BorderColor::all(ring);
            }
            None => {
                let e = commands
                    .spawn((RadarBlip(b.id), node, BackgroundColor(bg), BorderColor::all(ring), Pickable::IGNORE, ChildOf(square)))
                    .id();
                ents.0.insert(b.id, e);
            }
        }
    }
    // The camera's footprint: four thin white lines, turned to the sides of the trapezoid.
    let side = Vec2::new(
        match square_node.width { Val::Px(w) if w > 0.0 => w, _ => RADAR_SIDE },
        match square_node.height { Val::Px(h) if h > 0.0 => h, _ => RADAR_SIDE },
    ) - Vec2::splat(2.0);
    let lines: Vec<(Vec2, f32, f32)> = match feed.camera {
        Some(c) => (0..4)
            .map(|i| {
                let (a, b) = (map.to_square(c[i]) * side, map.to_square(c[(i + 1) % 4]) * side);
                let d = b - a;
                ((a + b) / 2.0, d.length(), d.y.atan2(d.x))
            })
            .collect(),
        None => Vec::new(),
    };
    let have: Vec<(Entity, usize)> = outline.iter().map(|(e, c)| (e, c.0)).collect();
    for (e, i) in &have {
        if *i >= lines.len() {
            commands.entity(*e).despawn();
        }
    }
    for (i, (mid, len, angle)) in lines.iter().enumerate() {
        let node = Node {
            position_type: PositionType::Absolute,
            left: Val::Px(mid.x - len / 2.0),
            top: Val::Px(mid.y - 0.75),
            width: Val::Px(*len),
            height: Val::Px(1.5),
            ..default()
        };
        let tf = UiTransform::from_rotation(Rot2::radians(*angle));
        match have.iter().find(|(_, j)| *j == i) {
            Some((e, _)) => {
                if let Ok((mut n, mut t)) = lines_q.get_mut(*e) {
                    *n = node;
                    *t = tf;
                }
            }
            None => {
                commands.spawn((RadarCamera(i), node, tf, BackgroundColor(Color::WHITE), Pickable::IGNORE, ChildOf(square)));
            }
        }
    }
}

/// Light the dot of the filter in force.
pub fn paint_filters(theme: Res<Theme>, state: Res<RadarState>, mut dots: Query<(&FilterDot, &mut BackgroundColor, &mut BorderColor)>) {
    for (d, mut bg, mut border) in &mut dots {
        let lit = (if d.station { state.stations } else { state.ships }) == d.state;
        let c = dot_colour(&theme, d.state, lit);
        let fill = if d.state == Filter::All { Color::NONE } else { c };
        if bg.0 != fill {
            bg.0 = fill;
        }
        *border = BorderColor::all(c);
    }
}

/// Pick a filter state; the pure part of a click on a dot or a name.
pub fn set_filter(state: &mut RadarState, station: bool, to: Option<Filter>) {
    let f = if station { &mut state.stations } else { &mut state.ships };
    *f = to.unwrap_or(f.next());
}

fn on_click(
    ev: On<PointerClick>,
    dots: Query<&FilterDot>,
    names: Query<&FilterName>,
    squares: Query<&RelativeCursorPosition, With<RadarSquare>>,
    mut state: ResMut<RadarState>,
    mut out: MessageWriter<RadarClick>,
) {
    let e = ev.entity;
    if let Ok(d) = dots.get(e) {
        set_filter(&mut state, d.station, Some(d.state));
    } else if let Ok(n) = names.get(e) {
        set_filter(&mut state, n.station, None);
    } else if let (Ok(rel), Some(map)) = (squares.get(e), state.map)
        && let Some(p) = rel.normalized
    {
        let uv = (p + Vec2::splat(0.5)).clamp(Vec2::ZERO, Vec2::ONE);
        out.write(RadarClick { at: map.to_world(uv), button: ev.button });
    }
}

/// Holding the left button on the square keeps sending clicks: dragging moves the camera along
/// (`on_click` alone only sees the release).
fn drag(
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    squares: Query<&RelativeCursorPosition, With<RadarSquare>>,
    state: Res<RadarState>,
    mut held: Local<bool>,
    mut out: MessageWriter<RadarClick>,
) {
    let (Some(buttons), Ok(rel), Some(map)) = (buttons, squares.single(), state.map) else { return };
    if buttons.just_pressed(MouseButton::Left) {
        *held = rel.cursor_over;
    }
    if !buttons.pressed(MouseButton::Left) {
        *held = false;
    }
    if *held
        && let Some(p) = rel.normalized
    {
        let uv = (p + Vec2::splat(0.5)).clamp(Vec2::ZERO, Vec2::ONE);
        out.write(RadarClick { at: map.to_world(uv), button: PointerButton::Primary });
    }
}

pub struct RadarPlugin;

/// Radar UI is rebuilt and refreshed before the HUD positions its root.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RadarUi;

#[allow(clippy::type_complexity)]
fn rebuild_on_theme_change(
    mut commands: Commands,
    theme: Res<Theme>,
    profile: Option<Res<UiProfile>>,
    config: Option<Res<HudConfig>>,
    roots: Query<Option<&Children>, With<RadarRoot>>,
    mut blips: ResMut<BlipEntities>,
    mut previous: Local<Option<(Option<UiProfile>, Option<sta_content::GuiLayout>, f32)>>,
) {
    let now = (
        profile.as_ref().map(|p| **p),
        config.as_ref().and_then(|c| c.gui.clone()),
        config.as_ref().map_or(16.0 / 9.0, |c| c.aspect),
    );
    let changed = theme.is_changed() || previous.as_ref() != Some(&now);
    *previous = Some(now);
    if !changed {
        return;
    }
    for children in &roots {
        for child in children.into_iter().flatten() {
            commands.entity(*child).despawn();
        }
    }
    blips.0.clear();
}

impl Plugin for RadarPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ThemePlugin>() {
            app.add_plugins(ThemePlugin);
        }
        app.add_plugins(super::radar_fog::RadarFogPlugin)
            .init_resource::<RadarFeed>()
            .init_resource::<RadarState>()
            .init_resource::<BlipEntities>()
            .add_message::<RadarClick>()
            .add_observer(on_click)
            .add_systems(Startup, spawn_radar)
            .add_systems(
                Update,
                (
                    rebuild_on_theme_change,
                    spawn_radar,
                    feed_from_link.run_if(resource_exists::<SimLink>).run_if(not(resource_exists::<super::hud::ExternalFeed>)),
                    drag,
                    refresh,
                    paint_filters,
                ).chain().in_set(GameSet::Ui).in_set(RadarUi).after(ProfileSync),
            );
    }
}

// ---- frame runs --------------------------------------------------------------

/// `STA64_UI_RADAR=1`: draw the radar over the headless frames
/// (`--headless --frames`). An environment switch, not a flag: `app.rs` is
/// the INT lane's; INT-M3 can promote it.
pub fn frame_overlay_requested() -> bool {
    std::env::var("STA64_UI_RADAR").as_deref() == Ok("1")
}

/// Add the radar to a frame run: a 2D camera into the capture image `target`,
/// the UI scaled to the 1080-unit screen the theme is written for, and the
/// radar's root aimed at that camera.
pub fn add_frame_overlay(app: &mut App, target: Handle<Image>, frame_height: u32) {
    use bevy::camera::RenderTarget;
    app.add_plugins(RadarPlugin).insert_resource(UiScale(frame_height as f32 / 1080.0)).add_systems(
        Startup,
        move |mut commands: Commands| {
            let cam = commands
                .spawn((
                    Camera2d,
                    Camera { order: 1, clear_color: ClearColorConfig::None, ..default() },
                    RenderTarget::from(target.clone()),
                    Msaa::Off,
                ))
                .id();
            commands.insert_resource(FrameUiCamera(cam));
        },
    );
    app.add_systems(Update, aim_at_camera.before(GameSet::Ui));
}

#[derive(Resource)]
pub(crate) struct FrameUiCamera(pub(crate) Entity);

fn aim_at_camera(mut commands: Commands, cam: Option<Res<FrameUiCamera>>, roots: Query<Entity, (With<RadarRoot>, Without<UiTargetCamera>)>) {
    let Some(cam) = cam else { return };
    for e in &roots {
        commands.entity(e).insert(UiTargetCamera(cam.0));
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::configure_sets;
    use crate::render::fog::{FogGrid, FogState};
    use crate::ui::radar_fog::RadarFog;
    use sta_bench::parse::SceneFile;
    use sta_bench::run::{Bench, RunConfig};
    use sta_sim::command::{Command, CommandKind, Order};
    use sta_sim::fake::ViewLog;
    use sta_sim::{Vec3 as SimVec3, WorldSource};

    const SCENE: &str = include_str!("../../tests/fixtures/radar.ini");
    const ME: PlayerId = PlayerId(1);

    /// A recorded run of the radar scene: 90 ticks with the flagship ordered
    /// across the map, replayed from a `ViewLog`.
    fn log() -> (ViewLog, Arc<ContentDb>, EntityId) {
        let cfg = Arc::new(RunConfig::fake());
        let content = cfg.content.clone();
        let mut bench = Bench::build(cfg, &SceneFile::parse(SCENE).unwrap()).unwrap();
        let flagship = bench.resolve("flagship").unwrap();
        let order = Command {
            player: ME,
            seq: 0,
            kind: CommandKind::Order(Order::Move { units: vec![flagship], to: SimVec3::from_f32([1500.0, 0.0, 1500.0]), queue: false }),
        };
        let batches = (0..90).map(|t| if t == 0 { vec![order.clone()] } else { Vec::new() });
        (ViewLog::record(&mut bench.sim, batches), content, flagship)
    }

    fn feed_at(log: &mut ViewLog, content: &Arc<ContentDb>, ticks: usize) -> RadarFeed {
        log.rewind();
        for _ in 0..ticks {
            log.step(&[]);
        }
        RadarFeed { view: log.view(), player: ME, selected: Vec::new(), content: Some(content.clone()), camera: None }
    }

    /// The same view without fog: everything is seen.
    fn omniscient(mut feed: RadarFeed) -> RadarFeed {
        let mut v = (*feed.view).clone();
        v.visibility.clear();
        feed.view = Arc::new(v);
        feed
    }

    fn ships(b: &[Blip]) -> Vec<&Blip> {
        b.iter().filter(|b| b.kind == Kind::Ship).collect()
    }

    #[test]
    fn blips_come_from_the_view_log() {
        let (mut log, db, flagship) = log();
        let fogged = feed_at(&mut log, &db, 0);
        let feed = omniscient(fogged.clone());
        let state = RadarState::default();
        let map = RadarMap::fit(&feed.view).unwrap();
        // The sim's visibility shows: of the rival's three units only the
        // raider near the flagship is seen.
        let seen = blips(&fogged, &state, &map);
        assert_eq!((seen.len(), seen.iter().filter(|b| b.standing == Standing::Enemy).count()), (6, 1), "{seen:?}");
        let b = blips(&feed, &state, &map);
        assert_eq!(b.len(), 8, "{b:?}");
        assert_eq!(ships(&b).len(), 5);
        assert_eq!(b.iter().filter(|b| b.kind == Kind::Station).count(), 2);
        assert_eq!(b.iter().filter(|b| b.kind == Kind::Body).count(), 1);
        // Own side and rivals (the ownerless moon is a body).
        let count = |s| b.iter().filter(|b| b.standing == s && b.kind != Kind::Body).count();
        assert_eq!((count(Standing::Own), count(Standing::Enemy)), (4, 3));
        // Everything lands inside the square, z up the screen.
        assert!(b.iter().all(|b| (0.0..=1.0).contains(&b.at.x) && (0.0..=1.0).contains(&b.at.y)), "{b:?}");
        let at = |b: &[Blip], id| b.iter().find(|b| b.id == id).unwrap().at;
        let rival = b.iter().find(|b| b.kind == Kind::Station && b.standing == Standing::Enemy).unwrap().at;
        let flag = at(&b, flagship);
        assert!(rival.y < flag.y && rival.x > flag.x, "the rival base is north-east of the flagship");
        // The recorded flagship moves; its blip follows it.
        let later = omniscient(feed_at(&mut log, &db, 80));
        let b2 = blips(&later, &state, &map);
        let p1 = at(&b2, flagship);
        assert!(p1.x > flag.x && p1.y < flag.y, "{flag} -> {p1}");
    }

    #[test]
    fn filters_pick_by_standing() {
        let (mut log, db, _) = log();
        let feed = omniscient(feed_at(&mut log, &db, 0));
        let map = RadarMap::fit(&feed.view).unwrap();
        let mut state = RadarState { ships: Filter::Enemies, stations: Filter::Allies, ..default() };
        let b = blips(&feed, &state, &map);
        assert!(ships(&b).iter().all(|b| b.standing == Standing::Enemy) && ships(&b).len() == 2);
        let stations: Vec<_> = b.iter().filter(|b| b.kind == Kind::Station).collect();
        assert!(stations.len() == 1 && stations[0].standing == Standing::Own);
        // Bodies ignore the filters; nothing else is neutral here.
        state.ships = Filter::Neutral;
        state.stations = Filter::Neutral;
        let b = blips(&feed, &state, &map);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].kind, Kind::Body);
        // Clicking cycles as stock does.
        let mut s = RadarState::default();
        set_filter(&mut s, false, None);
        set_filter(&mut s, true, Some(Filter::Neutral));
        assert_eq!((s.ships, s.stations), (Filter::Allies, Filter::Neutral));
        assert_eq!(Filter::Neutral.next(), Filter::All);
    }

    #[test]
    fn dying_units_leave_the_radar_and_the_map_never_shrinks() {
        let (mut log, db, _) = log();
        let mut feed = omniscient(feed_at(&mut log, &db, 0));
        let mut view = (*feed.view).clone();
        view.units[0].flags = view.units[0].flags | UnitFlags::DYING;
        feed.view = Arc::new(view);
        let map = RadarMap::fit(&feed.view).unwrap();
        assert_eq!(blips(&feed, &RadarState::default(), &map).len(), 7);

        let small = RadarMap { min: Vec2::new(0.0, 0.0), size: 1000.0 };
        let far = RadarMap { min: Vec2::new(2000.0, -500.0), size: 1000.0 };
        let both = small.grow(far);
        assert_eq!(both.grow(small), both);
        assert!(both.size >= 3000.0 && both.min == Vec2::new(0.0, -500.0));
        let p = Vec2::new(1234.0, 567.0);
        assert!(both.to_world(both.to_square(p)).distance(p) < 1e-3);
        // North is up: larger z is a smaller v.
        assert!(both.to_square(Vec2::new(0.0, 900.0)).y < both.to_square(Vec2::new(0.0, 100.0)).y);
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        configure_sets(&mut app);
        app.add_plugins(RadarPlugin);
        app.update();
        app
    }

    fn count<C: Component>(app: &mut App) -> usize {
        app.world_mut().query_filtered::<(), With<C>>().iter(app.world()).count()
    }

    #[test]
    fn theme_switch_rebuilds_ui_and_keeps_radar_data_and_fog() {
        let (mut log, db, flagship) = log();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<Assets<Image>>();
        configure_sets(&mut app);
        app.add_plugins(RadarPlugin);
        let map = RadarMap::of_bounds(Vec2::ZERO, Vec2::splat(2000.0));
        app.world_mut().resource_mut::<RadarState>().map = Some(map);
        app.world_mut().resource_mut::<RadarState>().map_fixed = true;
        app.world_mut().resource_mut::<RadarState>().ships = Filter::Enemies;
        app.world_mut().resource_mut::<RadarFeed>().clone_from(&omniscient(feed_at(&mut log, &db, 0)));
        app.world_mut().resource_mut::<RadarFeed>().selected = vec![flagship];
        app.insert_resource(FogState { grid: FogGrid::new([0.0, 0.0], [2000.0, 2000.0]), updates: 1 });
        app.update();
        let old_root = app.world_mut().query_filtered::<Entity, With<RadarRoot>>().single(app.world()).unwrap();
        let old_square = app.world_mut().query_filtered::<Entity, With<RadarSquare>>().single(app.world()).unwrap();
        let target = app.world_mut().spawn_empty().id();
        app.world_mut().entity_mut(old_root).insert(UiTargetCamera(target));
        assert!(count::<RadarFog>(&mut app) > 0);
        let old_blips = count::<RadarBlip>(&mut app);
        assert!(old_blips > 0);

        let new_theme = crate::ui::profile::UiProfile::Armada2.theme();
        app.world_mut().insert_resource(new_theme.clone());
        app.update();
        let new_root = app.world_mut().query_filtered::<Entity, With<RadarRoot>>().single(app.world()).unwrap();
        let new_square = app.world_mut().query_filtered::<Entity, With<RadarSquare>>().single(app.world()).unwrap();
        assert_eq!(old_root, new_root);
        assert_eq!(app.world().get::<UiTargetCamera>(new_root).unwrap().0, target);
        assert_ne!(old_square, new_square);
        assert_eq!(app.world().get::<BackgroundColor>(new_square).unwrap().0, mix(new_theme.panel, new_theme.track, 0.5));
        assert_eq!(app.world().get::<BorderColor>(new_square).unwrap().top, new_theme.accent2);
        assert_eq!(count::<RadarBlip>(&mut app), old_blips);
        assert_eq!(count::<RadarFog>(&mut app), 1);
        let state = app.world().resource::<RadarState>();
        assert_eq!((state.ships, state.map, state.map_fixed), (Filter::Enemies, Some(map), true));
        assert_eq!(app.world().resource::<RadarFeed>().selected, vec![flagship]);
        assert!(app.world().resource::<BlipEntities>().0.values().all(|&entity| app.world().get::<RadarBlip>(entity).is_some()));
    }

    #[test]
    fn the_panel_has_a_square_two_strips_and_a_blip_per_unit() {
        let (mut log, db, flagship) = log();
        let mut app = app();
        // The kit's panel and the radar's pieces.
        assert_eq!(count::<RadarRoot>(&mut app), 1);
        assert_eq!(count::<super::super::theme::Panel>(&mut app), 1);
        assert_eq!(count::<RadarSquare>(&mut app), 1);
        assert_eq!(count::<FilterDot>(&mut app), 8);
        assert_eq!(count::<FilterName>(&mut app), 2);
        assert_eq!(count::<RadarBlip>(&mut app), 0);

        let mut feed = omniscient(feed_at(&mut log, &db, 0));
        feed.selected = vec![flagship];
        feed.camera = Some([Vec2::new(500.0, 500.0), Vec2::new(1500.0, 500.0), Vec2::new(1500.0, 1200.0), Vec2::new(500.0, 1200.0)]);
        *app.world_mut().resource_mut::<RadarFeed>() = feed;
        app.update();
        assert_eq!(count::<RadarBlip>(&mut app), 8);
        assert_eq!(count::<RadarCamera>(&mut app), 4);

        let theme = app.world().resource::<Theme>().clone();
        let ent = *app.world().resource::<BlipEntities>().0.get(&flagship).unwrap();
        let node = app.world().get::<Node>(ent).unwrap().clone();
        // Own ship, selected: green, 7 units, white ring.
        assert_eq!((node.width, node.height), (Val::Px(SHIP_DOT), Val::Px(SHIP_DOT)));
        assert_eq!(node.border, UiRect::all(Val::Px(2.0)));
        assert_eq!(app.world().get::<BackgroundColor>(ent).unwrap().0, theme.good);
        assert_eq!(app.world().get::<BorderColor>(ent).unwrap().top, Color::WHITE);

        // Pick "Enemies" for ships: the filter and the dots follow.
        let dot = app
            .world_mut()
            .query::<(Entity, &FilterDot)>()
            .iter(app.world())
            .find(|(_, d)| !d.station && d.state == Filter::Enemies)
            .unwrap()
            .0;
        assert_eq!(app.world().get::<BorderColor>(dot).unwrap().top, dot_colour(&theme, Filter::Enemies, false));
        set_filter(&mut app.world_mut().resource_mut::<RadarState>(), false, Some(Filter::Enemies));
        app.update();
        assert_eq!(count::<RadarBlip>(&mut app), 5, "2 enemy ships, 2 stations, the moon");
        assert_eq!(app.world().get::<BorderColor>(dot).unwrap().top, theme.bad, "the lit dot is the full colour");

        // Everything leaves the view: the blips go.
        app.world_mut().resource_mut::<RadarFeed>().view = Arc::new(WorldView::default());
        app.update();
        assert_eq!(count::<RadarBlip>(&mut app), 0);
    }

    fn stock_config() -> HudConfig {
        let cfg = "\
screenWidth = 1600
screenHeight = 1200
minimapPanelArea = 0 800 400 400
minimapBackgroundPanel = \"map_panel\"
minimapBackgroundPanelSize = 2
minimapBackgroundPanel_0 = 0 0 400 400
minimapBackgroundPanel_1 = 2 3 20 30
minimapBackgroundSprite = \"map_background\"
minimapDisplayArea = 10 20 320 280
minimapShipFilterArea = 4 305 120 30
minimapStationFilterArea = 140 305 120 30
minimapShipFilterButton = \"ship_art\"
minimapStationFilterButton = \"station_art\"
";
        let (parsed, warnings) = sta_content::GuiCfg::parse("radar.cfg", cfg);
        assert!(warnings.is_empty(), "{warnings:?}");
        HudConfig { gui: Some(parsed.layout((1600, 1200))), aspect: 16.0 / 9.0, ..default() }
    }

    #[test]
    fn stock_rectangles_art_filters_and_state_survive_rebuild() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).init_resource::<Assets<Image>>();
        configure_sets(&mut app);
        app.insert_resource(UiProfile::Armada2);
        app.insert_resource(stock_config());
        app.add_plugins(RadarPlugin);
        let map = RadarMap::of_bounds(Vec2::ZERO, Vec2::splat(2000.0));
        app.world_mut().resource_mut::<RadarState>().map = Some(map);
        app.world_mut().resource_mut::<RadarState>().map_fixed = true;
        app.world_mut().resource_mut::<RadarState>().ships = Filter::Enemies;
        app.world_mut().resource_mut::<RadarFeed>().camera = Some([
            Vec2::new(500.0, 500.0),
            Vec2::new(1500.0, 500.0),
            Vec2::new(1500.0, 1500.0),
            Vec2::new(500.0, 1500.0),
        ]);
        app.insert_resource(FogState { grid: FogGrid::new([0.0, 0.0], [2000.0, 2000.0]), updates: 1 });
        app.update();

        let root = app.world_mut().query_filtered::<Entity, With<RadarRoot>>().single(app.world()).unwrap();
        let square = app.world_mut().query_filtered::<Entity, With<RadarSquare>>().single(app.world()).unwrap();
        let node = app.world().get::<Node>(square).unwrap();
        assert_eq!((node.left, node.top, node.width, node.height),
            (Val::Px(12.0), Val::Px(18.0), Val::Px(384.0), Val::Px(252.0)));
        assert_eq!(count::<super::super::theme::Panel>(&mut app), 0);
        assert_eq!(count::<FilterDot>(&mut app), 0);
        assert_eq!(count::<FilterName>(&mut app), 2);
        assert_eq!(count::<GuiImage>(&mut app), 5);
        assert_eq!(count::<RadarFog>(&mut app), 1);
        let outline = app.world_mut().query::<(&RadarCamera, &Node)>().iter(app.world())
            .find(|(camera, _)| camera.0 == 0).map(|(_, node)| node.clone()).unwrap();
        assert_eq!((outline.left, outline.top, outline.width),
            (Val::Px(95.5), Val::Px(186.75), Val::Px(191.0)));
        let names: Vec<_> = app.world_mut().query::<&GuiImage>().iter(app.world())
            .map(|image| (image.names[0].clone(), image.frame)).collect();
        assert!(names.contains(&("map_panel".to_owned(), 0)));
        assert!(names.contains(&("map_panel".to_owned(), 1)));
        assert!(names.contains(&("map_background".to_owned(), 0)));
        assert!(names.contains(&("ship_art".to_owned(), 0)));
        assert!(names.contains(&("station_art".to_owned(), 0)));

        let target = app.world_mut().spawn_empty().id();
        app.world_mut().entity_mut(root).insert(UiTargetCamera(target));
        app.world_mut().insert_resource(UiProfile::Lcars);
        app.update();
        assert_eq!(app.world_mut().query_filtered::<Entity, With<RadarRoot>>().single(app.world()).unwrap(), root);
        assert_ne!(app.world_mut().query_filtered::<Entity, With<RadarSquare>>().single(app.world()).unwrap(), square);
        assert_eq!(app.world().get::<UiTargetCamera>(root).unwrap().0, target);
        assert_eq!(count::<super::super::theme::Panel>(&mut app), 1);
        assert_eq!(count::<GuiImage>(&mut app), 0);
        assert_eq!(app.world().resource::<RadarState>().ships, Filter::Enemies);
        assert_eq!(app.world().resource::<RadarState>().map, Some(map));
    }

    #[test]
    fn stock_zero_filter_and_missing_panel_disable_only_their_chrome() {
        let mut config = stock_config();
        let gui = config.gui.as_mut().unwrap();
        gui.rects.iter_mut().find(|(key, _)| key.eq_ignore_ascii_case("minimapShipFilterArea")).unwrap().1.w = 0;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        configure_sets(&mut app);
        app.insert_resource(UiProfile::FleetOps);
        app.insert_resource(config);
        app.add_plugins(RadarPlugin);
        app.update();
        assert_eq!(count::<FilterName>(&mut app), 1);
        let root = app.world_mut().query_filtered::<Entity, With<RadarRoot>>().single(app.world()).unwrap();
        app.world_mut().resource_mut::<HudConfig>().gui.as_mut().unwrap().panels.get_mut("minimap").unwrap().area = None;
        app.update();
        assert_eq!(count::<RadarSquare>(&mut app), 0);
        let chrome = app.world().get::<Children>(root).unwrap()[0];
        assert_eq!(app.world().get::<Node>(chrome).unwrap().display, Display::None);
    }
}
