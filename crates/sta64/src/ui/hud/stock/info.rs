//! Stock resource and selection display. Only values exposed by `HudFeed` are drawn.

use bevy::picking::Pickable;
use bevy::prelude::*;
use sta_sim::{EntityId, Fixed};

use super::super::layout::R;
use super::super::model::{self, HudFeed, Info};
use super::super::panels::GroupTile;
use super::super::status::node_fraction;
use super::layout::{StockLayout, info_variant};
use crate::ui::gui_sprites::{GuiImage, GuiImageSampling};
use crate::ui::notices::Tooltip;
use crate::ui::sprite_font::{FontRole, SpriteText, SpriteTextStyle, sprite_text_styled};
use crate::ui::stock_font::{HAlign, StockTextCanvas, VAlign};
use crate::ui::theme::{Theme, UiButton, WidgetState};

#[path = "identity.rs"]
mod identity;

#[cfg(test)]
pub(in crate::ui::hud) use identity::{IdentityField, IdentityText};

#[derive(Component, Clone, Copy)]
pub(in crate::ui::hud) enum StockText {
    Resource(usize),
    Population(EntityId),
}

#[derive(Component, Clone, Copy)]
pub(in crate::ui::hud) enum StockBar {
    Hull(f32),
    Shield(f32),
    Progress(f32),
    /// Armada I's energy bar (UI-19).
    Energy(f32),
    Group(EntityId, f32),
    /// A group tile's shield bar (`infoMultiShipShield_<n>`).
    GroupShield(EntityId, f32),
    /// UI-48: a resource node's amount left over what it started with.
    Node(f32),
}

/// Use the authored planet-metal bar for metal-producing planets and the
/// dilithium bar geometry for other mineable nodes.
fn node_bar(def: &sta_content::UnitDef) -> Option<(&'static str, &'static str, &'static str)> {
    match super::super::status::node_slot(def)? {
        2 if def.planet_role == Some(sta_content::PlanetRole::Producer) => {
            Some(("infoSinglePlanetMetalBarArea", "infoSinglePlanetMetalBarDisableColor", "infoSinglePlanetMetalBarColor"))
        }
        _ => Some(("infoSingleDilithiumBarArea", "infoSingleDilithiumBarDisableColor", "infoSingleDilithiumBarColor")),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_node_bar(
    commands: &mut Commands, root: Entity, layout: &StockLayout, origin: Vec2,
    def: &sta_content::UnitDef, fallback: Option<R>, theme: &Theme,
) {
    let Some((area, track, fill)) = node_bar(def) else { return };
    let Some(r) = layout.at(origin, area).or_else(|| layout.raw_rect(area).is_none().then_some(fallback).flatten()) else { return };
    let slot = super::super::status::node_slot(def).expect("node bar has a resource");
    // The authored dilithium fill is for slot 0. Other resources sharing the
    // rectangle retain their own palette if the game has no dedicated panel.
    let fill_keys: &[&str] = if slot == 0 || area == "infoSinglePlanetMetalBarArea" { &[fill] } else { &[] };
    let fallback_tint = resource_colour(layout, slot + 2, theme.accent);
    cfg_bar(commands, root, layout, r, 1.0, (&[track], fill_keys, fallback_tint), StockBar::Node(r.w), area.trim_end_matches("Area"));
}

/// UI-48: a system icon of the single-selection card, by cfg slot (`infoSystemIcon_<n>`: 0
/// sensors, 1 shield generator, 2 weapons, 3 engines, 4 life support; the slots' meaning is
/// the stock cfgs' comments, read from data).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::hud) struct SystemIcon(pub usize);

#[derive(Component)]
pub(in crate::ui::hud) struct SystemHealth(usize);

/// The colour-bearing entity of a hull bar without a CFG fill colour.
#[derive(Component)]
pub(in crate::ui::hud) struct FallbackHullTint(StockBar);

/// The sim's system index ([`sta_sim::combat::damage::SystemView`] order: engines, weapons,
/// shield generator, life support, sensors) of cfg slot `n`.
const SLOT_SYSTEM: [usize; 5] = [4, 2, 1, 0, 3];

/// CFG keys use the A2FOExtensions thresholds. Stock Armada II uses a green
/// icon, a quarter-green base plus a three-quarter-green wedge for damage,
/// and a fifth-intensity neutral base when off (read from the executable,
/// static). Blend the two green values by health for STA64's single tint.
pub(super) fn system_tint(layout: Option<&StockLayout>, health: sta_sim::Fixed, on: bool) -> Color {
    let h = health.to_f32().clamp(0.0, 1.0);
    let key = if !on {
        "systemIconDisabledColor"
    } else if h <= 0.0 {
        "systemIconDestroyedColor"
    } else if h > 0.5 {
        "systemIconHealthyColor"
    } else if h > 0.25 {
        "systemIconLowColor"
    } else {
        "systemIconCriticalColor"
    };
    let fallback = if !on {
        Color::srgb(0.2, 0.2, 0.2)
    } else if h == 1.0 {
        Color::srgb(0.0, 1.0, 0.0)
    } else {
        Color::srgb(0.0, 0.25 + 0.5 * h, 0.0)
    };
    layout.and_then(|l| l.colour(key)).unwrap_or(fallback)
}

/// CFG slots follow the HUD's order. Units absent from the feed are whole and on.
fn slot_status(status: Option<&crate::ui::hud::status::StatusFeed>, id: EntityId, slot: usize) -> (Fixed, bool) {
    let n = SLOT_SYSTEM[slot];
    status.and_then(|s| s.systems.iter().find(|v| v.entity == id))
        .map_or((Fixed::ONE, true), |v| (v.health[n], v.on[n]))
}

fn health_label(health: Fixed) -> String {
    let percent = (health.to_f32().clamp(0.0, 1.0) * 100.0).round() as u32;
    percent.to_string()
}

fn system_readout_rect(layout: &StockLayout, origin: Vec2, builder: bool, slot: usize, icon: Option<R>) -> Option<R> {
    let build = format!("infoBuildSystem_{slot}");
    let single = format!("infoSystem_{slot}");
    let key = if builder && layout.raw_rect(&build).is_some() { &build } else { &single };
    if layout.raw_rect(key).is_some() {
        layout.at(origin, key)
    } else {
        icon.map(|r| R::new(r.x + r.w + 2.0, r.y, r.w * 2.0, r.h))
    }
}

pub(super) fn node(r: R) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(r.x),
        top: Val::Px(r.y),
        width: Val::Px(r.w),
        height: Val::Px(r.h),
        ..default()
    }
}

pub(super) fn place(origin: Vec2, r: R) -> R {
    R::new(origin.x + r.x, origin.y + r.y, r.w, r.h)
}

pub(super) fn picture(commands: &mut Commands, root: Entity, r: R, name: &str, frame: usize) {
    commands
        .entity(root)
        .with_child((node(r), Pickable::IGNORE, GuiImage::new(name).with_frame(frame)));
}

pub(super) fn text(commands: &mut Commands, root: Entity, r: R, value: String, color: Color, size: f32, kind: Option<StockText>) -> Entity {
    // Name and class lines and the numbers beside the bars: left, vertically centred (the
    // original's rule for these elements; grade in `stock_font::StockPlace`).
    let style = SpriteTextStyle::default().stock_label().placed(HAlign::Left, VAlign::Centre);
    text_styled(commands, root, r, value, color, size, kind, style)
}

/// A resource readout: the number is centred, both ways, in its cfg text rectangle, not
/// top-left anchored with a baseline lift. Armada II stock: measured on one original
/// 1280x720 frame, the digits' ink centre lies within 1 px of the rectangle's centre on
/// both axes (rectangles scaled per axis from the sheet). Armada I: not measured (its
/// rectangle sits centred in its panel, which suggests the same rule); TODO.md has the check.
pub(super) fn readout(commands: &mut Commands, root: Entity, r: R, value: String, color: Color, size: f32, kind: Option<StockText>) -> Entity {
    let style = SpriteTextStyle::default().stock_label().centered_in_box().placed(HAlign::Centre, VAlign::Centre);
    text_styled(commands, root, r, value, color, size, kind, style)
}

#[allow(clippy::too_many_arguments)]
fn text_styled(commands: &mut Commands, root: Entity, r: R, value: String, color: Color, size: f32, kind: Option<StockText>, mut style: SpriteTextStyle) -> Entity {
    if let Some(place) = style.place.as_mut() {
        place.rect = Vec2::new(r.w, r.h);
    }
    // SpriteFontPlugin replaces the SpriteText entity's Node while resolving glyphs.
    // Keep authored placement on a separate parent that it never rewrites.
    let mut wrap_node = node(r);
    // The centring relies on the SpriteText node keeping taffy rounding on (no LayoutConfig):
    // `use_rounding: false` there would bring back half-pixel blur.
    if style.in_box {
        wrap_node.align_items = AlignItems::Center;
        wrap_node.justify_content = JustifyContent::Center;
    }
    let wrapper = commands.spawn((wrap_node, Pickable::IGNORE)).id();
    let size = if style.min_size { size.max(18.0) } else { size };
    let mut e = commands.spawn(sprite_text_styled(value, size, color, style));
    if let Some(kind) = kind {
        e.insert(kind);
    }
    let child = e.id();
    commands.entity(wrapper).add_child(child);
    commands.entity(root).add_child(wrapper);
    child
}

/// Keeps a free-text name (a map's or a script's, up to the buffer's length) inside its authored
/// rect: the placement parent of the text clips what does not fit, so it cannot run over
/// neighbouring panels. Only the horizontal axis is clipped (the glyph line may be taller than
/// the authored rect, so a vertical clip would cut every name); `vertical` also clips the other
/// axis, for a wrapped name whose extra lines would spill over what is below.
pub(super) fn clip_to_rect(commands: &mut Commands, text: Entity, vertical: bool) {
    commands.queue(move |world: &mut World| {
        let Some(parent) = world.get::<ChildOf>(text).map(ChildOf::parent) else { return };
        // The original never clips text to its rectangle: on the stock text path (Armada II
        // family) a long free-text name runs on, as there. The clip stays for every other path.
        if world.get::<SpriteTextStyle>(text).is_some_and(|s| s.place.is_some())
            && world.get_resource::<crate::ui::sprite_font::SpriteFonts>().is_some_and(|f| f.stock_family)
            && world.get_resource::<StockTextCanvas>().is_some_and(|c| c.0.is_some())
        {
            return;
        }
        if let Some(mut node) = world.get_mut::<Node>(parent) {
            node.overflow = if vertical { Overflow::clip() } else { Overflow::clip_x() };
        }
    });
}

/// A group tile name longer than this wraps in the stock tile font and is clipped on both axes.
const TILE_NAME_WRAPS_FROM: usize = 16;

/// The unfilled part of a stock bar when the cfg names no colour for it.
const BAR_TRACK: Color = Color::srgba(0.0, 0.0, 0.0, 0.5);

/// The colours of a stock bar as `(track, fill)` (UI-48), from the first of
/// `tracks` and of `fills` the cfg sets (full key names, so each key the HUD
/// reads is written out where it is used). The original
/// draws the whole bar in `<stem>DisableColor`, then the filled part over it
/// in the bar's own colour, which for bars with a fixed colour is
/// `<stem>Color` (read from the stock Armada II executable, static; not
/// compared by eye). A key the cfg does not set leaves the fallback.
pub(super) fn bar_colours(layout: &StockLayout, tracks: &[&str], fills: &[&str], fill: Color) -> (Color, Color) {
    let first = |keys: &[&str]| keys.iter().find_map(|key| layout.colour(key));
    (first(tracks).unwrap_or(BAR_TRACK), first(fills).unwrap_or(fill))
}

pub(super) fn bar(commands: &mut Commands, root: Entity, r: R, value: f32, (track, color): (Color, Color), kind: StockBar) -> Entity {
    let fill = commands.spawn((
        kind,
        Node {
            width: Val::Px(r.w * value.clamp(0.0, 1.0)),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(color),
        Pickable::IGNORE,
    )).id();
    commands.spawn((
        node(r),
        BackgroundColor(track),
        Pickable::IGNORE,
        ChildOf(root),
    )).add_child(fill);
    fill
}

/// The stock Armada II health gradient and shield blue, read from the
/// executable, static. These fill colours also apply when the CFG names a sprite.
fn native_hull_tint(health: f32) -> Color {
    let health = health.clamp(0.0, 1.0);
    Color::srgb(1.0 - health, health, 0.0)
}

const STOCK_SHIELD_TINT: Color = Color::srgb(0.0, 0.0, 1.0);

#[allow(clippy::too_many_arguments)]
pub(super) fn cfg_bar(commands: &mut Commands, root: Entity, layout: &StockLayout, r: R, value: f32, (tracks, fills, fallback): (&[&str], &[&str], Color), kind: StockBar, sprite_key: &str) {
    let (track, fill) = bar_colours(layout, tracks, fills, fallback);
    let fill_entity = match layout.name(sprite_key) {
        Some(sprite) => sprite_bar(commands, root, r, value, (track, fill), kind, sprite),
        None => bar(commands, root, r, value, (track, fill), kind),
    };
    if matches!(kind, StockBar::Hull(_) | StockBar::Group(..))
        && fills.iter().all(|key| layout.colour(key).is_none())
    {
        commands.entity(fill_entity).insert(FallbackHullTint(kind));
    }
}

pub(super) fn sprite_bar(commands: &mut Commands, root: Entity, r: R, value: f32, (track, color): (Color, Color), kind: StockBar, sprite: &str) -> Entity {
    let track = commands.spawn((
        node(r),
        BackgroundColor(track),
        Pickable::IGNORE,
        ChildOf(root),
    )).id();
    let clip = commands.spawn((
        kind,
        Node { width: Val::Px(r.w * value.clamp(0.0, 1.0)), height: Val::Percent(100.0), overflow: Overflow::clip(), ..default() },
        Pickable::IGNORE,
        ChildOf(track),
    )).id();
    commands.spawn((
        Node { width: Val::Px(r.w), height: Val::Percent(100.0), flex_shrink: 0.0, ..default() },
        Pickable::IGNORE,
        GuiImage::new(sprite).with_color(color),
        ChildOf(clip),
    )).id()
}

pub(in crate::ui::hud) fn update_hull_tints(
    feed: Res<HudFeed>,
    mut fills: Query<(&FallbackHullTint, Option<&mut BackgroundColor>, Option<&mut GuiImage>)>,
) {
    if fills.is_empty() {
        return;
    }
    let selected = feed.selected_units();
    for (kind, bg, image) in &mut fills {
        let unit = match kind.0 {
            StockBar::Hull(_) => selected.first().copied(),
            StockBar::Group(id, _) => feed.view.unit(id),
            _ => None,
        };
        let Some(unit) = unit else { continue };
        let tint = native_hull_tint(model::frac(unit.hull));
        if let Some(mut image) = image {
            if image.color != tint {
                image.color = tint;
            }
        } else if let Some(mut bg) = bg
            && bg.0 != tint
        {
            bg.0 = tint;
        }
    }
}

fn resource_slot(id: usize) -> Option<usize> {
    match id {
        2 => Some(0),
        3 => Some(1),
        4 => Some(2),
        5 => Some(3),
        _ => None,
    }
}

fn whole(v: Fixed) -> i64 {
    v.to_bits() / Fixed::ONE.to_bits()
}

fn resource_text(feed: &HudFeed, id: usize) -> Option<String> {
    let player = feed.view.player(feed.player)?;
    if id == 0 {
        return player.crew.map(|crew| whole(crew).to_string());
    }
    if id == 1 {
        let [used, cap] = player.officers;
        return Some(format!("{}/{}", whole(used), whole(cap)));
    }
    let slot = resource_slot(id)?;
    player.resources.get(slot).map(|v| whole(*v).to_string())
}

/// Stock A2's resource palette (read from the executable, static). Roots' authored
/// resource_N_color overrides take priority, including white or black.
fn resource_colour(layout: &StockLayout, id: usize, fallback: Color) -> Color {
    if let Some(colour) = layout.colour(&format!("resource_{id}_color")) {
        return colour;
    }
    let [r, g, b] = match id {
        0 => [0.0, 1.0, 0.0],
        1 => [1.0, 1.0, 1.0],
        2 => [0.6, 0.9, 1.0],
        3 => [1.0, 1.0, 0.0],
        4 => [0.8, 0.8, 0.8],
        5 => [1.0, 0.0, 0.0],
        _ => return fallback,
    };
    Color::srgb(r, g, b)
}

fn info_background(variant: usize) -> &'static str {
    match variant {
        0 => "infoLowBackgroundPanel",
        1 => "infoMiddleBackgroundPanel",
        _ => "infoBackgroundPanel",
    }
}

/// An authored disabled builder rectangle must not revive the single layout.
fn chosen_rect(layout: &StockLayout, origin: Vec2, builder: bool, builder_key: &str, single_key: &str) -> Option<R> {
    if builder && layout.raw_rect(builder_key).is_some() {
        layout.at(origin, builder_key)
    } else {
        layout.at(origin, single_key)
    }
}

pub(super) fn wireframe_rect(layout: &StockLayout, origin: Vec2, builder: bool) -> Option<R> {
    if builder {
        for key in ["infoBuildWireframeIconArea", "infoBuildWireframe"] {
            if layout.raw_rect(key).is_some() {
                return layout.at(origin, key);
            }
        }
    }
    layout.at(origin, "infoSingleWireframeIconArea")
}

fn model_stem(model: &str) -> String {
    let name = model.rsplit(['/', '\\']).next().unwrap_or(model).to_ascii_lowercase();
    match name.rsplit_once('.') {
        Some((stem, ext)) if ["sod", "glb", "gltf", "m3d"].contains(&ext) => stem.to_owned(),
        _ => name,
    }
}

/// Fleet Ops names the single-selection systems backdrop after the equipped
/// model. The GUI sprite painter leaves it transparent when the SPR entry or
/// its texture is absent, preserving the stock icon/value layout.
fn system_background(commands: &mut Commands, root: Entity, layout: &StockLayout, origin: Vec2, model: &str) -> Option<Entity> {
    let r = layout.at(origin, "infoSingleSystemsDisplay")?;
    let stem = model_stem(model);
    if stem.is_empty() { return None; }
    Some(commands.spawn((node(r), Pickable::IGNORE, GuiImage::new(format!("{stem}_si")), ChildOf(root))).id())
}

#[derive(Component)]
pub(in crate::ui::hud) struct BackdropFallback(Entity);

pub(in crate::ui::hud) fn update_backdrop_fallback(
    pictures: Query<(), With<ImageNode>>,
    mut fallbacks: Query<(&BackdropFallback, &mut Visibility)>,
) {
    for (BackdropFallback(backdrop), mut visibility) in &mut fallbacks {
        let next = if pictures.contains(*backdrop) { Visibility::Hidden } else { Visibility::Inherited };
        if *visibility != next {
            *visibility = next;
        }
    }
}

fn wireframe_names(model: &str, layer: usize) -> Vec<String> {
    let stem = model_stem(model);
    if stem.is_empty() || !(1..=5).contains(&layer) {
        return Vec::new();
    }
    let mut names = vec![format!("{stem}w{layer}")];
    if layer == 1 {
        names.push(format!("{stem}_s"));
    }
    names
}

/// Native Wireframe::Render draws five independent GUI SPR layers in one square.
pub(super) fn wireframe_layers(commands: &mut Commands, root: Entity, r: R, model: &str) {
    wireframe_layers_of(commands, root, r, model, None, None);
}

/// UI-28: a wireframe layer of a unit on the panel, tinted by its system's state each frame.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::hud) struct WireLayer {
    pub unit: EntityId,
    /// 1..=5, the `w<n>` sprite.
    pub layer: usize,
}

/// The sim's system index ([`sta_sim::combat::damage::SystemView`] order) of wireframe layer `w<n>`:
/// w1 weapons, w2 engines, w3 life support, w4 shield generator, w5 sensors (read from the stock
/// wireframe art by eye; the original draws layer n with the ship's system n-1).
const LAYER_SYSTEM: [usize; 5] = [1, 0, 3, 2, 4];

/// A wireframe layer's colour (A2 `WireframeIcon::DrawSystem`, read from the executable, static):
/// white; with damage left `r` above 0.01, `(r, 0.2(1 - r), 0.2(1 - r))`, pure red when destroyed;
/// a switched-off system yellow (the original flashes it; here it is steady, ours).
pub(super) fn wire_tint(health: Fixed, on: bool) -> Color {
    if !on {
        return Color::srgb(1.0, 1.0, 0.0);
    }
    let r = (1.0 - health.to_f32()).clamp(0.0, 1.0);
    if r > 0.01 { Color::srgb(r, 0.2 * (1.0 - r), 0.2 * (1.0 - r)) } else { Color::WHITE }
}

fn layer_tint(status: Option<&crate::ui::hud::status::StatusFeed>, w: WireLayer) -> Color {
    let Some(&n) = LAYER_SYSTEM.get(w.layer.wrapping_sub(1)) else { return Color::WHITE };
    status.and_then(|s| s.systems.iter().find(|v| v.entity == w.unit)).map_or(Color::WHITE, |v| wire_tint(v.health[n], v.on[n]))
}

/// [`wireframe_layers`] for a unit whose layers follow its systems' state (`None`: plain art).
fn wireframe_layers_of(commands: &mut Commands, root: Entity, r: R, model: &str, unit: Option<EntityId>, backdrop: Option<Entity>) {
    let stem = model_stem(model);
    if stem.is_empty() {
        return;
    }
    for layer in 1..=5 {
        let mut image = GuiImage::new("").with_sampling(GuiImageSampling::Linear);
        image.names = wireframe_names(&stem, layer);
        let mut e = commands.spawn((node(r), Pickable::IGNORE, image, ChildOf(root)));
        if let Some(unit) = unit {
            e.insert(WireLayer { unit, layer });
        }
        if let Some(backdrop) = backdrop {
            e.insert(BackdropFallback(backdrop));
        }
    }
}

/// Armada I's GUI cfgs (read from data: `gui_fed/kli/rom/bor.cfg`) have no `resourcePanelArea`;
/// each readout is its own panel keyed `<n>PanelArea`, with `<n>BackgroundPanel[Size|_0]`,
/// `<n>Icon` + `<n>IconArea` and `<n>TextArea` (the last two relative to the panel).
/// Used only when the single resource panel is absent. The stock texts show crew and
/// dilithium as plain whole numbers and officers as "in use / available" (the
/// `GUI_RD_OFFICERS_VTOOLTIP` string says so); the number format itself is assumed.
const SEPARATE_RESOURCES: [(&str, usize, &str); 3] = super::SEPARATE_RESOURCES;

fn spawn_separate_resources(commands: &mut Commands, root: Entity, layout: &StockLayout, feed: &HudFeed, theme: &Theme) {
    for (prefix, id, flag) in SEPARATE_RESOURCES {
        let Some(area) = layout.panel(prefix) else {
            continue;
        };
        // Same convention as the single panel: an authored non-zero flag hides it.
        if layout.number(flag).is_some_and(|n| n != 0.0) {
            continue;
        }
        let origin = Vec2::new(area.x, area.y);
        for piece in layout.pieces_in(prefix, &format!("{prefix}BackgroundPanel")) {
            picture(commands, root, place(origin, piece.rect), &piece.name, piece.frame);
        }
        if let Some(name) = layout.name(&format!("{prefix}Icon"))
            && let Some(r) = layout.at(origin, &format!("{prefix}IconArea"))
        {
            picture(commands, root, r, name, 0);
        }
        let Some(r) = layout.at(origin, &format!("{prefix}TextArea")) else {
            continue;
        };
        let value = resource_text(feed, id).unwrap_or_default();
        readout(commands, root, r, value, theme.text, theme.small_size, Some(StockText::Resource(id)));
    }
}

pub(super) fn spawn_resource(commands: &mut Commands, root: Entity, layout: &StockLayout, feed: &HudFeed, theme: &Theme) {
    if super::separate_resources(layout) {
        spawn_separate_resources(commands, root, layout, feed, theme);
        return;
    }
    let Some(area) = layout.panel("resource") else {
        return;
    };
    let origin = Vec2::new(area.x, area.y);
    for piece in layout.pieces_in("resource", "resourcePanel") {
        picture(commands, root, place(origin, piece.rect), &piece.name, piece.frame);
    }
    // CFG resource IDs are different from the shared four-slot stock vector.
    for (id, flag) in [
        (0, "noCrew"),
        (1, "noOfficers"),
        (2, "noDilithium"),
        (3, "noLatinum"),
        (4, "noMetal"),
        (5, "noBiomatter"),
    ] {
        if layout.number(flag).is_some_and(|n| n != 0.0) {
            continue;
        }
        // Keep the node even before a player arrives; `update` fills it later.
        let value = resource_text(feed, id).unwrap_or_default();
        let Some(r) = layout.at(origin, &format!("resource_{id}")) else {
            continue;
        };
        let color = resource_colour(layout, id, theme.text);
        readout(
            commands,
            root,
            r,
            value,
            color,
            theme.small_size,
            Some(StockText::Resource(id)),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_single(
    commands: &mut Commands,
    root: Entity,
    layout: &StockLayout,
    feed: &HudFeed,
    theme: &Theme,
    origin: Vec2,
    single: &model::Single,
) {
    let def = feed.view.unit(single.id).and_then(|u| feed.content.as_deref().and_then(|db| db.unit(u.def)));
    let is_node = def.is_some_and(|d| super::super::status::node_slot(d).is_some());
    let backdrop = def.and_then(|d| system_background(commands, root, layout, origin, &d.model));
    if let (Some(backdrop), Some(def), Some(db)) = (backdrop, def, feed.content.as_deref()) {
        super::weapon_icons::spawn(commands, root, layout, origin, backdrop, def, db);
    }
    if let Some(r) = chosen_rect(
        layout,
        origin,
        single.builder,
        "infoBuildName",
        "infoSingleNameTextArea",
    ) {
        let name = text(
            commands,
            root,
            r,
            single.name.clone(),
            layout
                .colour("infoSingleNameTextColor")
                .or_else(|| layout.colour("nameTextColor"))
                .or_else(|| layout.colour("infoTextColor"))
                .unwrap_or(theme.text),
            theme.small_size,
            None,
        );
        clip_to_rect(commands, name, false);
    }
    if let Some(r) = chosen_rect(
        layout,
        origin,
        single.builder,
        "infoBuildClass",
        "infoSingleClassTextArea",
    ) {
        text(
            commands,
            root,
            r,
            single.class.clone(),
            layout
                .colour("infoSingleClassTextColor")
                .or_else(|| layout.colour("classTextColor"))
                .or_else(|| layout.colour("infoTextColor"))
                .unwrap_or(theme.text_dim),
            theme.micro_size,
            None,
        );
    }
    identity::spawn(commands, root, layout, theme, origin, single);
    if let Some(r) = wireframe_rect(layout, origin, single.builder)
        && let Some(def) = def
    {
        wireframe_layers_of(commands, root, r, &def.model, Some(single.id), backdrop);
    }
    let hull_rect = chosen_rect(
        layout,
        origin,
        single.builder,
        "infoBuildHullBar",
        "infoSingleHullBarArea",
    );
    if !is_node && let Some(r) = hull_rect {
        let fills: &[&str] = if single.builder { &["infoBuildHullBarColor", "infoSingleHullBarColor"] } else { &["infoSingleHullBarColor"] };
        cfg_bar(commands, root, layout, r, single.hull, (&["infoSingleHullBarDisableColor", "infoSingleShieldBarDisableColor"], fills, native_hull_tint(single.hull)), StockBar::Hull(r.w), "infoSingleHullBar");
    }
    if let Some(shield) = single.shield
        && let Some(r) = chosen_rect(
            layout,
            origin,
            single.builder,
            "infoBuildShieldBar",
            "infoSingleShieldBarArea",
        )
    {
        let fills: &[&str] = if single.builder { &["infoBuildShieldBarColor", "infoSingleShieldBarColor"] } else { &["infoSingleShieldBarColor"] };
        cfg_bar(commands, root, layout, r, shield, (&["infoSingleShieldBarDisableColor"], fills, STOCK_SHIELD_TINT), StockBar::Shield(r.w), "infoSingleShieldBar");
    }
    if let Some(progress) = single.progress
        && let Some(r) = layout.at(origin, "infoSingleConstructionBarArea")
    {
        bar(commands, root, r, progress, bar_colours(layout, &["infoSingleConstructionBarDisableColor"], &["infoSingleConstructionBarColor"], theme.warn), StockBar::Progress(r.w));
    }
    if let Some(def) = def {
        spawn_node_bar(commands, root, layout, origin, def, hull_rect, theme);
    }
    if let Some(population) = super::super::status::population_text(feed, single.id) {
        // Reuse alignment from the planet owner row when present. A separate
        // line preserves the CFG's resource bar and title geometry.
        let anchor = layout.at(origin, "infoSinglePlanetTeamTextArea")
            .or_else(|| layout.at(origin, "infoSingleClassTextArea"));
        if let Some(r) = anchor {
            let line = R::new(r.x, r.bottom() + 3.0, r.w.max(190.0 * layout.scale_x), r.h);
            let entity = text(commands, root, line, population, theme.text, theme.micro_size, Some(StockText::Population(single.id)));
            clip_to_rect(commands, entity, false);
        }
    }
    // The stock engine gives the system picture and health percentage their
    // own rectangles. Read both from the mounted CFG.
    if let Some(def) = def.filter(|d| d.system_hit_points.iter().any(|hp| *hp > Fixed::ZERO)) {
        for (slot, system) in SLOT_SYSTEM.into_iter().enumerate() {
            let icon = chosen_rect(layout, origin, single.builder, &format!("infoBuildSystemIcon_{slot}"), &format!("infoSystemIcon_{slot}"));
            if let Some(r) = icon {
                commands.entity(root).with_child((node(r), Pickable::IGNORE, SystemIcon(slot), GuiImage::new("infosystem").with_frame(slot)));
            }
            if def.system_hit_points[system] > Fixed::ZERO
                && let Some(r) = system_readout_rect(layout, origin, single.builder, slot, icon)
            {
                let label = text_styled(
                    commands, root, r, String::new(), theme.text, r.h * 0.8, None,
                    SpriteTextStyle { tint_all: true, ..default() }.centered_in_box(),
                );
                commands.entity(label).insert(SystemHealth(slot));
                clip_to_rect(commands, label, false);
            }
        }
    }

}

#[allow(clippy::too_many_arguments)]
fn spawn_group(
    commands: &mut Commands,
    root: Entity,
    layout: &StockLayout,
    feed: &HudFeed,
    theme: &Theme,
    origin: Vec2,
    variant: usize,
    tiles: &[model::Tile],
) {
    let small = variant == 2;
    for (i, tile) in tiles.iter().enumerate() {
        let key = if small {
            format!("infoMultiShipIconSm_{i}")
        } else {
            format!("infoMultiShipIcon_{i}")
        };
        let Some(r) = layout.at(origin, &key) else {
            continue;
        };
        let entity = commands
            .spawn((
                GroupTile(tile.id),
                UiButton,
                WidgetState::Normal,
                Tooltip::new(tile.name.clone()),
                node(r),
            ))
            .id();
        // Bevy's Node inserts a default BackgroundColor; omit it so the shared
        // widget painter cannot tint the transparent wireframe hit area.
        commands.entity(entity).remove::<BackgroundColor>();
        commands.entity(root).add_child(entity);
        let wireframe_key = if small {
            format!("infoMultiShipWireframeSm_{i}")
        } else {
            format!("infoMultiShipWireframe_{i}")
        };
        let wireframe = if layout.raw_rect(&wireframe_key).is_some() {
            layout.at(origin, &wireframe_key)
                .map(|area| R::new(area.x - r.x, area.y - r.y, area.w, area.h))
        } else {
            Some(R::new(0.0, 0.0, r.w, r.h))
        };
        if let Some(wireframe) = wireframe
            && let Some(unit) = feed.view.unit(tile.id)
            && let Some(def) = feed.content.as_deref().and_then(|db| db.unit(unit.def))
        {
            wireframe_layers_of(commands, entity, wireframe, &def.model, Some(tile.id), None);
        }
        if !small {
            // Existing incomplete CFGs retain the icon inset when no name key exists.
            let fallback = R::new(2.0, 2.0, (r.w - 4.0).max(0.0), (r.h - 4.0).max(0.0));
            let name_rect = layout.at(origin, &format!("infoMultiShipName_{i}"))
                .map_or(fallback, |name| R::new(name.x - r.x, name.y - r.y, name.w, name.h));
            let name = text_styled(
                commands,
                entity,
                name_rect,
                tile.name.clone(),
                theme.text,
                theme.micro_size,
                None,
                group_label_style(layout, name_rect.w),
            );
            clip_to_rect(commands, name, tile.name.chars().count() > TILE_NAME_WRAPS_FROM);
        }
        let hull_key = if small {
            format!("infoMultiShipHullSm_{i}")
        } else {
            format!("infoMultiShipHull_{i}")
        };
        if let Some(hull) = layout.at(origin, &hull_key) {
            cfg_bar(commands, root, layout, hull, tile.hull, (&["infoMultiHullBarDisableColor", "infoMultiShieldBarDisableColor"], &["infoMultiHullBarColor"], native_hull_tint(tile.hull)), StockBar::Group(tile.id, hull.w), "infoMultiHullBar");
        }
        // UI-48: the tile's shield bar (A2 and FO author one per tile; FO's compact layout `Sm`).
        let shield_key = if small { format!("infoMultiShipShieldSm_{i}") } else { format!("infoMultiShipShield_{i}") };
        if let (Some(shield), Some(r)) = (tile.shield, layout.at(origin, &shield_key)) {
            cfg_bar(commands, root, layout, r, shield, (&["infoMultiShieldBarDisableColor"], &["infoMultiShieldBarColor"], STOCK_SHIELD_TINT), StockBar::GroupShield(tile.id, r.w), "infoMultiShieldBar");
        }
    }
}

/// The CFG's information font names a stock role when it matches one of the
/// mounted sprite fonts. Compact names honor micro size as the HUD scales.
fn group_label_style(layout: &StockLayout, width: f32) -> SpriteTextStyle {
    let role = layout.name("InformationFont").and_then(FontRole::from_name).unwrap_or(FontRole::Small);
    // Stock text (declared-material filter); `scaled` keeps its own size, so no role rule.
    SpriteTextStyle { scaled: true, tint_all: true, stock: Some(crate::ui::stock_font::StockRole::Default), ..SpriteTextStyle::role(role).wrapped(width) }
}

pub(super) fn spawn(commands: &mut Commands, root: Entity, layout: &StockLayout, feed: &HudFeed, theme: &Theme) {
    spawn_resource(commands, root, layout, feed, theme);
    let info = model::info(feed);
    let variant = info_variant(&info);
    let Some(area) = layout.info_area(variant) else {
        return;
    };
    let origin = Vec2::new(area.x, area.y);
    for piece in layout.pieces_from(info_background(variant), layout.info_canvas_origin(variant)) {
        picture(commands, root, place(origin, piece.rect), &piece.name, piece.frame);
    }
    match &info {
        Info::None => {}
        Info::Single(single) => spawn_single(commands, root, layout, feed, theme, origin, single),
        Info::Group(tiles) => spawn_group(commands, root, layout, feed, theme, origin, variant, tiles),
    }
}

#[allow(clippy::too_many_arguments)] // Bevy injects these system parameters.
pub(in crate::ui::hud) fn update(
    feed: Res<HudFeed>,
    status: Option<Res<crate::ui::hud::status::StatusFeed>>,
    mut texts: Query<(&StockText, &mut SpriteText)>,
    mut bars: Query<(&StockBar, &mut Node)>,
    mut crew: Query<&mut SpriteText, (With<super::a1::info::CrewText>, Without<StockText>)>,
    mut health_labels: Query<(&SystemHealth, &mut SpriteText), (Without<StockText>, Without<super::a1::info::CrewText>)>,
    mut icons: Query<(&SystemIcon, &mut GuiImage), Without<WireLayer>>,
    mut layers: Query<(&WireLayer, &mut GuiImage), Without<SystemIcon>>,
    state: Option<Res<super::StockState>>,
) {
    for (w, mut image) in &mut layers {
        let tint = layer_tint(status.as_deref(), *w);
        if image.color != tint {
            image.color = tint;
        }
    }
    let info = model::info(&feed);
    if let Info::Single(s) = &info {
        let layout = state.as_deref().and_then(|st| st.layout.as_ref());
        for (icon, mut image) in &mut icons {
            let (health, on) = slot_status(status.as_deref(), s.id, icon.0);
            let tint = system_tint(layout, health, on);
            if image.color != tint {
                image.color = tint;
            }
        }
        for (slot, mut text) in &mut health_labels {
            let (health, on) = slot_status(status.as_deref(), s.id, slot.0);
            let label = health_label(health);
            // Stock Armada II draws functioning system percentages in green
            // and permanently disabled ones in dim grey. The feed has no
            // native timed-repair overlay, so a failed damaged system is red.
            let tint = if on {
                Color::srgb(0.0, 1.0, 0.0)
            } else if health < Fixed::ONE {
                Color::srgb(1.0, 0.0, 0.0)
            } else {
                Color::srgb(0.2, 0.2, 0.2)
            };
            if text.text != label {
                text.text = label;
            }
            if text.color != tint {
                text.color = tint;
            }
        }
    }
    let extra = super::a1::info::extra(&feed);
    for mut text in &mut crew {
        let value = extra.as_ref().and_then(|e| e.crew).map(|(now, _)| now.to_string()).unwrap_or_default();
        if text.text != value {
            text.text = value;
        }
    }
    for (kind, mut text) in &mut texts {
        let value = match *kind {
            StockText::Resource(id) => resource_text(&feed, id),
            StockText::Population(id) => super::super::status::population_text(&feed, id),
        }.unwrap_or_default();
        if text.text != value {
            text.text = value;
        }
    }
    for (kind, mut node) in &mut bars {
        let (value, width) = match (kind, &info) {
            (StockBar::Hull(w), Info::Single(s)) => (s.hull, *w),
            (StockBar::Shield(w), Info::Single(s)) => (s.shield.unwrap_or(0.0), *w),
            (StockBar::Progress(w), Info::Single(s)) => (s.progress.unwrap_or(0.0), *w),
            (StockBar::Energy(w), Info::Single(_)) => (extra.as_ref().and_then(|e| e.energy).unwrap_or(0.0), *w),
            (StockBar::Node(w), Info::Single(s)) => (node_fraction(status.as_deref(), s.id), *w),
            (StockBar::Group(id, w), Info::Group(tiles)) => (
                tiles.iter().find(|tile| tile.id == *id).map_or(0.0, |tile| tile.hull),
                *w,
            ),
            (StockBar::GroupShield(id, w), Info::Group(tiles)) => (
                tiles.iter().find(|tile| tile.id == *id).and_then(|tile| tile.shield).unwrap_or(0.0),
                *w,
            ),
            _ => continue,
        };
        let next = Val::Px(width * value.clamp(0.0, 1.0));
        if node.width != next {
            node.width = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::world::CommandQueue;
    use sta_content::gui_cfg::{GuiLayout, Rect};
    use sta_sim::fake::ScriptedSource;
    use sta_sim::{
        Fixed, PlayerId, TeamId, WorldSource,
        view::{PlayerView, WorldView},
    };
    use std::sync::Arc;

    /// UI-48: a stock bar's track and fill come from the cfg when it names them.
    #[test]
    fn bar_colours_come_from_the_cfg_and_fall_back_per_key() {
        use sta_content::gui_cfg::Colour;
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        let colour = |r: f32, g: f32, b: f32| Colour { r, g, b, a: 1.0 };
        cfg.colours.insert("infoSingleConstructionBarColor".into(), colour(1.0, 0.6, 0.2));
        cfg.colours.insert("infoSingleConstructionBarDisableColor".into(), colour(0.3, 0.3, 0.3));
        cfg.colours.insert("infosingleshieldbardisablecolor".into(), colour(0.2, 0.2, 0.2));
        cfg.colours.insert("infoMultiShieldBarDisableColor".into(), colour(0.1, 0.1, 0.1));
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let fallback = Color::srgb(0.0, 1.0, 0.0);
        // Both keys set.
        assert_eq!(bar_colours(&layout, &["infoSingleConstructionBarDisableColor"], &["infoSingleConstructionBarColor"], fallback), (Color::srgba(0.3, 0.3, 0.3, 1.0), Color::srgba(1.0, 0.6, 0.2, 1.0)));
        // Only the track is set (key case does not matter): the fill keeps the fallback.
        assert_eq!(bar_colours(&layout, &["infoSingleShieldBarDisableColor"], &["infoSingleShieldBarColor"], fallback), (Color::srgba(0.2, 0.2, 0.2, 1.0), fallback));
        // Neither: the built-in track and the fallback fill.
        assert_eq!(bar_colours(&layout, &["infoSingleHullBarDisableColor"], &["infoSingleHullBarColor"], fallback), (BAR_TRACK, fallback));
        // The first key the cfg sets wins.
        assert_eq!(bar_colours(&layout, &["infoMultiHullBarDisableColor", "infoMultiShieldBarDisableColor"], &["infoMultiHullBarColor"], fallback), (Color::srgba(0.1, 0.1, 0.1, 1.0), fallback));
    }

    /// A long free-text name (a mod map can carry up to the buffer's length) stays inside its
    /// authored rect: the placement parent of the single card's name and of a group tile's name
    /// clips. Invented 127-character name.
    #[test]
    fn a_long_given_name_is_clipped_to_its_authored_rect() {
        let long = "Wanderer ".repeat(15)[..127].to_owned();
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        cfg.rects.insert("infoSingleNameTextArea".into(), Rect { x: 10, y: 10, w: 120, h: 12 });
        cfg.rects.insert("infoMultiShipIcon_0".into(), Rect { x: 10, y: 20, w: 40, h: 40 });
        cfg.rects.insert("infoMultiShipName_0".into(), Rect { x: 10, y: 61, w: 40, h: 9 });
        cfg.rects.insert("infoMultiShipIcon_1".into(), Rect { x: 60, y: 20, w: 40, h: 40 });
        cfg.rects.insert("infoMultiShipName_1".into(), Rect { x: 60, y: 61, w: 40, h: 9 });
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let theme = Theme::default();
        let feed = HudFeed::default();
        let single = model::Single {
            id: EntityId(1),
            name: long.clone(),
            captain: None,
            registry: None,
            class: "Class".into(),
            own: true,
            hull: 1.0,
            hull_now: 10,
            hull_max: 10,
            shield: None,
            progress: None,
            builder: false,
        };
        let tile = model::Tile { id: EntityId(1), name: long, hull: 1.0, shield: None };
        let short = model::Tile { id: EntityId(2), name: "Short".into(), hull: 1.0, shield: None };
        let mut world = World::new();
        let root = world.spawn(Node::default()).id();
        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, &world);
            spawn_single(&mut commands, root, &layout, &feed, &theme, Vec2::ZERO, &single);
            spawn_group(&mut commands, root, &layout, &feed, &theme, Vec2::ZERO, 1, &[tile.clone(), short]);
        }
        queue.apply(&mut world);
        let mut texts = world.query::<(&SpriteText, &ChildOf)>();
        let mut overflow = |pred: &dyn Fn(&str) -> bool| -> Vec<Overflow> {
            texts.iter(&world).filter(|(t, _)| pred(&t.text)).filter_map(|(_, parent)| world.get::<Node>(parent.parent()).map(|n| n.overflow)).collect()
        };
        // The single card's name: horizontal clip only (the stock look keeps the line's height).
        // The long wrapped tile name: both axes; a short tile name: horizontal only.
        assert_eq!(overflow(&|t| t.len() >= 100), vec![Overflow::clip_x(), Overflow::clip()]);
        assert_eq!(overflow(&|t| t == "Short"), vec![Overflow::clip_x()]);
    }

    #[test]
    fn resource_ids_follow_native_stock_order() {
        assert_eq!(
            (0..6).map(resource_slot).collect::<Vec<_>>(),
            [None, None, Some(0), Some(1), Some(2), Some(3)]
        );
    }

    #[test]
    fn information_background_tracks_native_variant() {
        assert_eq!(info_background(0), "infoLowBackgroundPanel");
        assert_eq!(info_background(1), "infoMiddleBackgroundPanel");
        assert_eq!(info_background(2), "infoBackgroundPanel");
    }

    #[test]
    fn builder_zero_rectangle_suppresses_single_fallback() {
        let mut cfg = GuiLayout {
            canvas: (1600, 1200),
            ..Default::default()
        };
        cfg.rects.insert(
            "infoSingleNameTextArea".into(),
            Rect {
                x: 4,
                y: 5,
                w: 30,
                h: 10,
            },
        );
        let origin = Vec2::new(100.0, 200.0);
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        assert_eq!(
            chosen_rect(&layout, origin, true, "infoBuildName", "infoSingleNameTextArea"),
            Some(R::new(104.0, 205.0, 30.0, 10.0))
        );
        cfg.rects
            .insert("infoBuildName".into(), Rect { x: 8, y: 8, w: 0, h: 0 });
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        assert_eq!(
            chosen_rect(&layout, origin, true, "infoBuildName", "infoSingleNameTextArea"),
            None
        );
    }

    #[test]
    fn native_wireframe_names_are_layered() {
        assert_eq!(
            wireframe_names("models/Fed_ConSt.SOD", 1),
            ["fed_constw1", "fed_const_s"]
        );
        assert_eq!(wireframe_names("models/Fed_ConSt.SOD", 5), ["fed_constw5"]);
    }

    #[test]
    fn system_backdrop_uses_model_sprite_and_only_an_authored_visible_rect() {
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        cfg.rects.insert("infoSingleSystemsDisplay".into(), Rect { x: 10, y: 18, w: 512, h: 128 });
        let mut world = World::new();
        let root = world.spawn(Node::default()).id();
        let mut queue = CommandQueue::default();
        {
            let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
            let mut commands = Commands::new(&mut queue, &world);
            assert!(system_background(&mut commands, root, &layout, Vec2::new(30.0, 40.0), "models/Fed_ConSt.SOD").is_some());
        }
        queue.apply(&mut world);
        let child = world.get::<Children>(root).unwrap()[0];
        let image = world.get::<GuiImage>(child).unwrap();
        assert_eq!(image.names, ["fed_const_si"]);
        let node = world.get::<Node>(child).unwrap();
        assert_eq!((node.left, node.top, node.width, node.height),
            (Val::Px(40.0), Val::Px(58.0), Val::Px(512.0), Val::Px(128.0)));

        cfg.rects.insert("infoSingleSystemsDisplay".into(), Rect { x: 10, y: 18, w: 0, h: 128 });
        let disabled = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let absent = StockLayout::new(&GuiLayout::default(), Vec2::new(1600.0, 1200.0));
        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, &world);
            assert!(system_background(&mut commands, root, &disabled, Vec2::ZERO, "Fed_ConSt.SOD").is_none());
            assert!(system_background(&mut commands, root, &absent, Vec2::ZERO, "Fed_ConSt.SOD").is_none());
        }
        queue.apply(&mut world);
        assert_eq!(world.get::<Children>(root).unwrap().len(), 1);
    }

    #[test]
    fn system_backdrop_hides_the_wireframe_only_when_its_image_resolves() {
        let mut app = App::new();
        app.add_systems(Update, update_backdrop_fallback);
        let backdrop = app.world_mut().spawn_empty().id();
        let fallback = app.world_mut().spawn((BackdropFallback(backdrop), Visibility::Inherited)).id();
        app.update();
        assert_eq!(app.world().get::<Visibility>(fallback), Some(&Visibility::Inherited));
        app.world_mut().entity_mut(backdrop).insert(ImageNode::default());
        app.update();
        assert_eq!(app.world().get::<Visibility>(fallback), Some(&Visibility::Hidden));
        app.world_mut().entity_mut(backdrop).remove::<ImageNode>();
        app.update();
        assert_eq!(app.world().get::<Visibility>(fallback), Some(&Visibility::Inherited));
    }

    #[test]
    fn stock_wireframe_layers_opt_into_linear_sampling() {
        let mut world = World::new();
        let root = world.spawn(Node::default()).id();
        let rect = R::new(12.0, 18.0, 64.0, 64.0);
        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, &world);
            wireframe_layers(&mut commands, root, rect, "models/Fed_ConSt.SOD");
        }
        queue.apply(&mut world);
        let children = world.get::<Children>(root).unwrap();
        assert_eq!(children.len(), 5);
        for (index, entity) in children.iter().enumerate() {
            let image = world.get::<GuiImage>(entity).unwrap();
            assert_eq!(image.sampling, GuiImageSampling::Linear);
            assert_eq!(image.names, wireframe_names("fed_const", index + 1));
            let node = world.get::<Node>(entity).unwrap();
            assert_eq!((node.left, node.top, node.width, node.height),
                (Val::Px(12.0), Val::Px(18.0), Val::Px(64.0), Val::Px(64.0)));
        }
    }

    #[test]
    fn group_tile_keeps_full_name_and_transparent_hit_area() {
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        cfg.rects.insert("infoMultiShipIcon_0".into(), Rect { x: 10, y: 20, w: 100, h: 60 });
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let feed = HudFeed::default();
        let theme = Theme::default();
        let name = "USS Long Ship Name";
        let tile = model::Tile { id: EntityId(7), name: name.into(), hull: 1.0, shield: None };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        crate::app::configure_sets(&mut app);
        app.add_plugins(crate::ui::theme::ThemePlugin);
        let root = app.world_mut().spawn(Node::default()).id();
        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, app.world());
            spawn_group(&mut commands, root, &layout, &feed, &theme, Vec2::ZERO, 1, &[tile]);
        }
        queue.apply(app.world_mut());
        let entity = app.world_mut().query_filtered::<Entity, With<GroupTile>>().single(app.world()).unwrap();
        assert!(app.world().get::<UiButton>(entity).is_some());
        assert!(app.world().get::<Tooltip>(entity).is_some());
        assert!(app.world().get::<Node>(entity).is_some());
        assert!(app.world().get::<BackgroundColor>(entity).is_none());
        for state in [WidgetState::Normal, WidgetState::Hovered, WidgetState::Pressed, WidgetState::Disabled] {
            *app.world_mut().get_mut::<WidgetState>(entity).unwrap() = state;
            app.update();
            assert!(app.world().get::<BackgroundColor>(entity).is_none(), "painted {state:?} tile");
        }
        let mut labels = app.world_mut().query::<(&SpriteText, &SpriteTextStyle, &ChildOf)>();
        let label = labels
            .iter(app.world())
            .find(|(_, _, parent)| app.world().get::<ChildOf>(parent.parent()).is_some_and(|p| p.parent() == entity))
            .unwrap();
        assert_eq!(label.0.text, name);
        assert_eq!(label.1.wrap, Some(96.0));
        assert_eq!(label.0.size, theme.micro_size);
        assert_eq!(label.1.role, Some(FontRole::Small));
        assert!(!label.1.native);
        assert!(label.1.scaled);
        assert!(!label.1.min_size);
        assert_eq!(app.world().get::<Tooltip>(entity).unwrap().short, name);
    }

    #[test]
    fn group_label_uses_recognized_information_font_role() {
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        cfg.names.insert("InformationFont".into(), "FontMedium".into());
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let style = group_label_style(&layout, 76.0);
        assert_eq!(style.role, Some(FontRole::Medium));
        assert_eq!(style.wrap, Some(76.0));
        assert!(!style.native);
        assert!(!style.min_size);

        cfg.names.insert("InformationFont".into(), "custom_font_alias".into());
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        assert_eq!(group_label_style(&layout, 76.0).role, Some(FontRole::Small));
    }

    #[test]
    fn group_name_uses_authored_rectangle_and_small_tiles_have_no_caption() {
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        cfg.rects.insert("infoMultiShipIcon_0".into(), Rect { x: 70, y: 90, w: 40, h: 40 });
        cfg.rects.insert("infoMultiShipName_0".into(), Rect { x: 50, y: 77, w: 90, h: 9 });
        cfg.rects.insert("infoMultiShipIconSm_0".into(), Rect { x: 70, y: 90, w: 40, h: 40 });
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let feed = HudFeed::default();
        let theme = Theme::default();
        for variant in [1, 2] {
            let mut world = World::new();
            let root = world.spawn(Node::default()).id();
            let mut queue = CommandQueue::default();
            {
                let mut commands = Commands::new(&mut queue, &world);
                let tile = model::Tile { id: EntityId(7), name: "USS Dunlop".into(), hull: 1.0, shield: None };
                spawn_group(&mut commands, root, &layout, &feed, &theme, Vec2::ZERO, variant, &[tile]);
            }
            queue.apply(&mut world);
            let label = world.query::<(&SpriteText, &ChildOf)>().iter(&world)
                .find(|(text, _)| text.text == "USS Dunlop");
            if variant == 2 {
                assert!(label.is_none());
                continue;
            }
            let (label, parent) = label.unwrap();
            assert_eq!(label.size, theme.micro_size);
            let wrapper = parent.parent();
            let tile = world.query_filtered::<Entity, With<GroupTile>>().single(&world).unwrap();
            assert_eq!(world.get::<ChildOf>(wrapper).unwrap().parent(), tile);
            let rect = world.get::<Node>(wrapper).unwrap();
            let expected = R::new(-20.0, -13.0, 90.0, 9.0);
            assert_eq!((rect.left, rect.top, rect.width, rect.height),
                (Val::Px(expected.x), Val::Px(expected.y), Val::Px(expected.w), Val::Px(expected.h)));
            let style = world.get::<SpriteTextStyle>(world.get::<Children>(wrapper).unwrap()[0]).unwrap();
            assert_eq!(style.wrap, Some(expected.w));
            assert!(!style.native && !style.min_size && style.scaled);
        }
    }

    #[test]
    fn seeded_group_tiers_use_authored_wireframes_and_only_normal_names() {
        let db = Arc::new(sta_content::fake::db());
        let units = (1..=30)
            .map(|id| format!("{{ id: {id}, def: 'fx_cruiser', owner: 1, hull: 100, rails: [{{ at: 0, pos: {{ x: 0, y: 0, z: 0 }} }}] }}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut source = ScriptedSource::from_json5(&format!("{{ units: [{units}] }}"), &db).unwrap();
        let feed = HudFeed {
            view: source.step(&[]).view,
            content: Some(db),
            ..default()
        };
        let tiles = (1..=30)
            .map(|id| model::Tile { id: EntityId(id), name: format!("Ship {id} full name"), hull: 1.0, shield: None })
            .collect::<Vec<_>>();
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        for i in 0..30 {
            // The normal CFG has 16 slots; the compact CFG has 30.
            for suffix in if i < 16 { &["", "Sm"][..] } else { &["Sm"][..] } {
                cfg.rects.insert(format!("infoMultiShipIcon{suffix}_{i}"), Rect { x: 10 + i as i32 * 50, y: 20, w: 40, h: 40 });
                cfg.rects.insert(format!("infoMultiShipWireframe{suffix}_{i}"), Rect { x: 14 + i as i32 * 50, y: 26, w: 25, h: 25 });
            }
            if i < 16 {
                cfg.rects.insert(format!("infoMultiShipName_{i}"), Rect { x: 10 + i as i32 * 50, y: 61, w: 40, h: 9 });
            }
        }
        // An explicit disabled wireframe stays disabled; an absent key uses the icon.
        cfg.rects.insert("infoMultiShipWireframeSm_1".into(), Rect { x: 0, y: 0, w: 0, h: 0 });
        cfg.rects.remove("infoMultiShipWireframeSm_2");
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let theme = Theme::default();
        for (variant, count) in [(1, 16), (2, 30)] {
            let mut world = World::new();
            let root = world.spawn(Node::default()).id();
            let mut queue = CommandQueue::default();
            {
                let mut commands = Commands::new(&mut queue, &world);
                spawn_group(&mut commands, root, &layout, &feed, &theme, Vec2::ZERO, variant, &tiles[..count]);
            }
            queue.apply(&mut world);
            let mut tile_query = world.query::<(&GroupTile, &Node, &Tooltip, Option<&BackgroundColor>, Option<&Children>)>();
            let rendered = tile_query.iter(&world).collect::<Vec<_>>();
            assert_eq!(rendered.len(), count);
            for (group, hit, tooltip, background, children) in rendered {
                let i = group.0.0 as usize - 1;
                assert_eq!(tooltip.short, tiles[i].name);
                assert!(background.is_none());
                assert_eq!((hit.width, hit.height), (Val::Px(40.0), Val::Px(40.0)));
                let frames = children.into_iter().flat_map(|c| c.iter()).filter_map(|child| world.get::<GuiImage>(child).map(|_| child)).collect::<Vec<_>>();
                if variant == 2 && i == 1 {
                    assert!(frames.is_empty());
                } else {
                    assert_eq!(frames.len(), 5);
                    let image_node = world.get::<Node>(frames[0]).unwrap();
                    let expected = if variant == 2 && i == 2 {
                        (Val::Px(0.0), Val::Px(0.0), Val::Px(40.0), Val::Px(40.0))
                    } else {
                        (Val::Px(4.0), Val::Px(6.0), Val::Px(25.0), Val::Px(25.0))
                    };
                    assert_eq!((image_node.left, image_node.top, image_node.width, image_node.height), expected);
                }
            }
            let labels = world.query::<&SpriteText>().iter(&world).count();
            assert_eq!(labels, if variant == 1 { 16 } else { 0 });
        }
    }

    /// Uses installed CFG rectangles and parsed sprite font sheets when opted in.
    #[test]
    fn installed_group_names_fit_one_line_above_hulls() {
        let Some(root) = std::env::var_os("STA64_TEST_INSTALL") else { return };
        let Ok(mode) = std::env::var("STA64_TEST_MOD") else { return };
        let cfg_name = match mode.as_str() {
            "STA2 Classic" | "STA2 Classic Mod" => "gui_fed.cfg",
            "FO4" | "Fleet Ops 4.0" => "gui_federation16x9.cfg",
            _ => return,
        };
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(&mode)).unwrap();
        let cfg = crate::game::gui_layout(&vfs, cfg_name).unwrap();
        let layout = StockLayout::new(&cfg, Vec2::new(1920.0, 1080.0));
        let origin = layout.info_origin(1);
        assert!(!crate::ui::sprite_font::read_fonts(&vfs).is_empty(), "{mode}: no parsed sprite fonts");
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(crate::ui::sprite_font::SpriteFontPlugin);
        app.init_resource::<Assets<Image>>();
        app.insert_resource(crate::viewer::Mounted(vfs));
        app.update();
        assert!(!app.world().resource::<crate::ui::sprite_font::SpriteFonts>().fonts.is_empty(), "{mode}: no supported sprite font sheets");
        for (i, ship) in ["USS Dunlop", "USS Hayduk"].into_iter().enumerate() {
            let name = layout.at(origin, &format!("infoMultiShipName_{i}")).unwrap();
            let hull = layout.at(origin, &format!("infoMultiShipHull_{i}")).unwrap();
            let label = app.world_mut().spawn(sprite_text_styled(
                ship, Theme::default().micro_size, Color::WHITE, group_label_style(&layout, name.w),
            )).id();
            let unwrapped = app.world_mut().spawn(sprite_text_styled(
                ship, Theme::default().micro_size, Color::WHITE, group_label_style(&layout, name.w * 10.0),
            )).id();
            app.update();
            let node = app.world().get::<Node>(label).unwrap();
            let (Val::Px(width), Val::Px(height)) = (node.width, node.height) else { panic!("{mode}: unresolved sprite text") };
            let Val::Px(single_line_height) = app.world().get::<Node>(unwrapped).unwrap().height else { panic!("{mode}: unresolved baseline") };
            eprintln!("{mode} {ship}: name={name:?}, hull={hull:?}, resolved={width}x{height}");
            assert!(width <= name.w, "{mode} {ship}: label width {width} > authored {}", name.w);
            assert_eq!(height, single_line_height, "{mode} {ship}: wrapped beyond one line");
            assert!(height <= hull.y - name.y, "{mode} {ship}: label height {height} reaches hull at {}", hull.y);
        }
    }

    #[test]
    fn resources_can_arrive_after_initial_feed() {
        let mut feed = HudFeed {
            player: PlayerId(1),
            ..default()
        };
        assert_eq!(resource_text(&feed, 1), None);
        assert_eq!(resource_text(&feed, 0), None);
        feed.view = Arc::new(WorldView {
            players: vec![PlayerView { crew: None,
                id: PlayerId(1),
                team: TeamId(1),
                resources: vec![Fixed::ZERO; 4],
                officers: [Fixed::ZERO; 2],
                eliminated: false,
                relations: Vec::new(),
            }],
            ..Default::default()
        });
        assert_eq!(resource_text(&feed, 1).as_deref(), Some("0/0"));
        assert_eq!(resource_text(&feed, 0), None);
        Arc::make_mut(&mut feed.view).players[0].officers = [Fixed::from_int(3), Fixed::from_int(10)];
        assert_eq!(resource_text(&feed, 1).as_deref(), Some("3/10"));
        Arc::make_mut(&mut feed.view).players[0].crew = Some(Fixed::ZERO);
        assert_eq!(resource_text(&feed, 0).as_deref(), Some("0"));
    }

    #[test]
    fn fixed_whole_values_truncate_toward_zero_without_float_rounding() {
        assert_eq!(whole(Fixed::from_bits(Fixed::ONE.to_bits() * 16_777_217)), 16_777_217);
        assert_eq!(whole(Fixed::from_bits(-Fixed::ONE.to_bits() - 1)), -1);
    }

    /// Rule on installed data, no values: every resource readout's text rectangle lies inside
    /// its panel (the single strip, or each separate panel of the Armada I family).
    /// `STA64_TEST_A1_INSTALL=<Armada I root>` mounts stock Armada I (no mod); `STA64_TEST_INSTALL`
    /// with an optional `STA64_TEST_MOD` mounts an Armada II family install. A set variable that
    /// cannot be mounted or read fails; an unset one skips with a message.
    #[test]
    fn installed_resource_text_rectangles_lie_inside_their_panels() {
        let a1 = std::env::var_os("STA64_TEST_A1_INSTALL");
        let a2 = std::env::var_os("STA64_TEST_INSTALL");
        if a1.is_none() && a2.is_none() {
            eprintln!("SKIP: set STA64_TEST_A1_INSTALL (Armada I root) or STA64_TEST_INSTALL (+ STA64_TEST_MOD)");
            return;
        }
        let mut mounts = Vec::new();
        if let Some(root) = a1 {
            mounts.push((root, None, vec!["gui_fed.cfg", "gui_kli.cfg", "gui_rom.cfg", "gui_bor.cfg"]));
        }
        if let Some(root) = a2 {
            let m = std::env::var("STA64_TEST_MOD").ok().filter(|m| !m.is_empty());
            mounts.push((root, m, vec!["gui_fed.cfg"]));
        }
        for (root, mode, cfgs) in mounts {
            let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), mode.as_deref())
                .unwrap_or_else(|e| panic!("cannot mount {root:?} ({mode:?}): {e:?}"));
            for cfg_name in cfgs {
                let cfg = crate::game::gui_layout(&vfs, cfg_name).unwrap_or_else(|| panic!("{cfg_name} missing"));
                for canvas in [Vec2::new(1024.0, 768.0), Vec2::new(2752.0, 1152.0)] {
                    let layout = StockLayout::new(&cfg, canvas);
                    let inside = |outer: R, inner: R| {
                        inner.x >= outer.x - 0.01
                            && inner.y >= outer.y - 0.01
                            && inner.right() <= outer.right() + 0.01
                            && inner.bottom() <= outer.bottom() + 0.01
                    };
                    let mut checked = 0;
                    if super::super::separate_resources(&layout) {
                        for (prefix, ..) in SEPARATE_RESOURCES {
                            let Some(panel) = layout.panel(prefix) else { continue };
                            let Some(text) = layout.at(Vec2::new(panel.x, panel.y), &format!("{prefix}TextArea")) else { continue };
                            assert!(inside(panel, text), "{cfg_name}: {prefix} text {text:?} outside panel {panel:?}");
                            checked += 1;
                        }
                    } else if let Some(panel) = layout.panel("resource") {
                        for id in 0..6 {
                            let Some(text) = layout.at(Vec2::new(panel.x, panel.y), &format!("resource_{id}")) else { continue };
                            assert!(inside(panel, text), "{cfg_name}: resource_{id} text {text:?} outside panel {panel:?}");
                            checked += 1;
                        }
                    }
                    assert!(checked > 0, "{cfg_name} {canvas:?}: no readout rectangle found");
                    eprintln!("{cfg_name} {canvas:?}: {checked} readout rectangles inside their panels");
                }
            }
        }
    }

    /// Readouts are centred in their rectangle on both axes, whatever its size and the
    /// window's (the rectangle is already in logical px); other stock text is not.
    #[test]
    fn readout_wrapper_centres_its_text_in_the_rectangle() {
        for r in [R::new(10.0, 4.0, 60.0, 12.0), R::new(500.0, 40.0, 300.0, 60.0)] {
            let mut world = World::new();
            let root = world.spawn(Node::default()).id();
            let mut queue = CommandQueue::default();
            {
                let mut commands = Commands::new(&mut queue, &world);
                readout(&mut commands, root, r, "10".into(), Color::WHITE, 12.0, Some(StockText::Resource(2)));
                text(&mut commands, root, r, "10".into(), Color::WHITE, 12.0, None);
            }
            queue.apply(&mut world);
            let mut found = Vec::new();
            for (node, kids) in world.query::<(&Node, &Children)>().iter(&world) {
                if node.position_type == PositionType::Absolute && node.left == Val::Px(r.x) {
                    let kid = kids[0];
                    let centred = world.get::<crate::ui::sprite_font::SpriteTextStyle>(kid).unwrap().in_box;
                    found.push((centred, node.align_items, node.justify_content, node.width, node.height));
                }
            }
            found.sort_by_key(|f| !f.0);
            assert_eq!(found.len(), 2);
            assert_eq!(found[0], (true, AlignItems::Center, JustifyContent::Center, Val::Px(r.w), Val::Px(r.h)));
            assert_ne!(found[1].1, AlignItems::Center);
            assert_ne!(found[1].2, JustifyContent::Center);
        }
    }

    /// Runs bevy_ui layout on an invented box with an odd leftover, at two scales: the readout's
    /// text is centred on both axes (within one physical pixel, the snapping), sits on a whole physical
    /// pixel, stays centred when the string changes, and the plain helper's wrapper is exactly
    /// the authored node.
    #[test]
    fn readout_text_is_centred_and_pixel_aligned_after_layout() {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo, Viewport};
        use bevy::ui::UiGlobalTransform;
        let r = R::new(13.0, 7.0, 61.0, 29.0);
        for scale in [1.0_f32, 4.0 / 3.0] {
            let mut app = App::new();
            app.add_plugins((
                MinimalPlugins,
                bevy::transform::TransformPlugin,
                bevy::asset::AssetPlugin::default(),
                bevy::window::WindowPlugin { primary_window: None, ..default() },
                bevy::input::InputPlugin,
                bevy::a11y::AccessibilityPlugin,
                bevy::picking::DefaultPickingPlugins,
                bevy::text::TextPlugin,
                bevy::ui::UiPlugin,
            ));
            app.init_resource::<Assets<Image>>();
            app.init_resource::<Assets<bevy::image::TextureAtlasLayout>>();
            app.insert_resource(UiScale(scale));
            app.world_mut().spawn((
                Camera2d,
                Camera {
                    computed: ComputedCameraValues {
                        target_info: Some(RenderTargetInfo { physical_size: UVec2::new(800, 600), scale_factor: 1.0 }),
                        ..default()
                    },
                    viewport: Some(Viewport { physical_size: UVec2::new(800, 600), ..default() }),
                    ..default()
                },
            ));
            let root = app.world_mut().spawn(Node { width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }).id();
            let mut queue = CommandQueue::default();
            let (readout_text, plain_text) = {
                let mut commands = Commands::new(&mut queue, app.world());
                let a = readout(&mut commands, root, r, "7".into(), Color::WHITE, 12.0, Some(StockText::Resource(2)));
                let b = text(&mut commands, root, r, "7".into(), Color::WHITE, 12.0, None);
                (a, b)
            };
            queue.apply(app.world_mut());
            let wrapper_of = |app: &App, e: Entity| app.world().get::<ChildOf>(e).unwrap().parent();
            // What `resolve_text` leaves: a fixed-size node of the glyph run (odd sizes).
            for width in [11.0, 41.0] {
                for e in [readout_text, plain_text] {
                    app.world_mut().entity_mut(e).insert(Node { width: Val::Px(width), height: Val::Px(15.0), flex_shrink: 0.0, ..default() });
                }
                app.update();
                let centre = |app: &App, e: Entity| app.world().get::<UiGlobalTransform>(e).unwrap().translation;
                let size = |app: &App, e: Entity| app.world().get::<ComputedNode>(e).unwrap().size;
                let wrapper = wrapper_of(&app, readout_text);
                let (wc, tc) = (centre(&app, wrapper), centre(&app, readout_text));
                // Whole-pixel snapping of both nodes' edges can leave one physical pixel at a fractional scale.
                assert!((wc.x - tc.x).abs() <= 1.0 && (wc.y - tc.y).abs() <= 1.0, "scale {scale} width {width}: wrapper {wc:?} text {tc:?}");
                let ts = size(&app, readout_text);
                let (left, top) = (tc.x - ts.x / 2.0, tc.y - ts.y / 2.0);
                assert!((left - left.round()).abs() < 1e-3 && (top - top.round()).abs() < 1e-3, "scale {scale}: text at {left},{top} off the pixel grid");
            }
            let plain_wrapper = app.world().get::<Node>(wrapper_of(&app, plain_text)).unwrap();
            assert_eq!(*plain_wrapper, node(r));
        }
    }

    #[test]
    fn sprite_text_node_replacement_preserves_authored_wrapper() {
        let mut world = World::new();
        let root = world.spawn(Node::default()).id();
        let authored = R::new(31.0, 42.0, 80.0, 20.0);
        let mut queue = CommandQueue::default();
        {
            let mut commands = Commands::new(&mut queue, &world);
            text(
                &mut commands,
                root,
                authored,
                "10".into(),
                Color::WHITE,
                12.0,
                Some(StockText::Resource(2)),
            );
        }
        queue.apply(&mut world);
        let inner = world
            .query_filtered::<Entity, With<StockText>>()
            .single(&world)
            .unwrap();
        assert_eq!(world.get::<SpriteText>(inner).unwrap().size, 18.0);
        // UI-16 / ADR 0007 T1: the mod's sprite font, never the scalable TTF override.
        assert!(world.get::<crate::ui::sprite_font::ScalableText>(inner).is_none());
        assert!(world.get::<Node>(inner).is_some());
        let wrapper = world.get::<ChildOf>(inner).unwrap().parent();
        assert_eq!(world.get::<ChildOf>(wrapper).unwrap().parent(), root);
        // SpriteFontPlugin::resolve_text replaces this inner Node with glyph dimensions.
        world.entity_mut(inner).insert(Node {
            width: Val::Px(24.0),
            height: Val::Px(12.0),
            ..default()
        });
        let wrapper_node = world.get::<Node>(wrapper).unwrap();
        assert_eq!(
            (
                wrapper_node.left,
                wrapper_node.top,
                wrapper_node.width,
                wrapper_node.height
            ),
            (Val::Px(31.0), Val::Px(42.0), Val::Px(80.0), Val::Px(20.0))
        );
    }

    /// UI-48: which node draws which bar, and the fraction it shows.
    #[test]
    fn a_node_shows_its_amount_left_on_the_dilithium_or_planet_metal_bar() {
        use sta_content::UnitDef;
        let fake = sta_content::fake::db();
        let moon = fake.units().find(|u| u.odf == "fx_moon").unwrap().clone();
        assert_eq!(node_bar(&moon).map(|b| b.0), Some("infoSingleDilithiumBarArea"));
        let mut planet = UnitDef::new("p", "planet");
        planet.planet_role = Some(sta_content::PlanetRole::Producer);
        planet.resource = Some(2);
        assert_eq!(node_bar(&planet).map(|b| b.0), Some("infoSinglePlanetMetalBarArea"));
        assert!(node_bar(&UnitDef::new("c", "craft")).is_none());
        let node = |amount: i32, infinite| sta_sim::economy::view::NodeView { entity: EntityId(5), resource: 0, amount: Fixed::from_int(amount), start: Fixed::from_int(400), infinite };
        let feed = |n| crate::ui::hud::status::StatusFeed { nodes: vec![n], ..Default::default() };
        assert_eq!(node_fraction(Some(&feed(node(100, false))), EntityId(5)), 0.25);
        assert_eq!(node_fraction(Some(&feed(node(100, true))), EntityId(5)), 1.0, "infinite: always full");
        let empty = sta_sim::economy::view::NodeView { start: Fixed::ZERO, ..node(0, false) };
        assert_eq!(node_fraction(Some(&feed(empty)), EntityId(5)), 0.0);
        assert_eq!(node_fraction(None, EntityId(5)), 1.0);
    }

    /// Subsystem values come from the sim in CFG slot order and respect each authored colour.
    #[test]
    fn system_icons_are_tinted_by_their_systems_state() {
        use sta_content::gui_cfg::{Colour, GuiLayout};
        use sta_sim::combat::damage::SystemView;
        let f = Fixed::from_f64;
        let view = SystemView { entity: EntityId(5), health: [f(0.125), Fixed::ONE, f(0.5), Fixed::ZERO, Fixed::ONE], on: [true, true, true, true, false] };
        let feed = crate::ui::hud::status::StatusFeed { systems: vec![view], ..Default::default() };
        let slot_tint = |layout, id, slot| {
            let (health, on) = slot_status(Some(&feed), id, slot);
            system_tint(layout, health, on)
        };
        assert_eq!(slot_tint(None, EntityId(5), 3), Color::srgb(0.0, 0.3125, 0.0));
        assert_eq!(slot_tint(None, EntityId(5), 1), Color::srgb(0.0, 0.5, 0.0));
        assert_eq!(slot_tint(None, EntityId(5), 4), Color::srgb(0.0, 0.25, 0.0));
        assert_eq!(slot_tint(None, EntityId(5), 0), Color::srgb(0.2, 0.2, 0.2));
        assert_eq!(slot_tint(None, EntityId(5), 2), Color::srgb(0.0, 1.0, 0.0));
        assert_eq!(slot_tint(None, EntityId(9), 3), Color::srgb(0.0, 1.0, 0.0), "an unlisted unit is whole");
        assert_eq!(health_label(slot_status(Some(&feed), EntityId(5), 1).0), "50");
        assert_eq!(health_label(slot_status(Some(&feed), EntityId(5), 4).0), "0");
        assert_eq!(health_label(slot_status(Some(&feed), EntityId(9), 0).0), "100");
        // With the palette (as Roots' STL mod sets it).
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        let c = |r, g, b| Colour { r, g, b, a: 1.0 };
        for (k, v) in [("systemIconHealthyColor", c(0.0, 0.25, 1.0)), ("systemIconLowColor", c(1.0, 0.9, 0.0)), ("systemIconCriticalColor", c(1.0, 0.15, 0.0)), ("systemIconDisabledColor", c(0.25, 0.55, 1.0)), ("systemIconDestroyedColor", c(1.0, 0.0, 1.0))] {
            cfg.colours.insert(k.into(), v);
        }
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let l = Some(&layout);
        assert_eq!(slot_tint(l, EntityId(5), 3), Color::srgba(1.0, 0.15, 0.0, 1.0), "critical");
        assert_eq!(slot_tint(l, EntityId(5), 1), Color::srgba(1.0, 0.9, 0.0, 1.0), "low");
        assert_eq!(slot_tint(l, EntityId(5), 4), Color::srgba(1.0, 0.0, 1.0, 1.0), "destroyed");
        assert_eq!(slot_tint(l, EntityId(5), 0), Color::srgba(0.25, 0.55, 1.0, 1.0), "disabled");
        assert_eq!(slot_tint(l, EntityId(5), 2), Color::srgba(0.0, 0.25, 1.0, 1.0), "healthy");
    }

    #[test]
    fn system_values_use_their_own_cfg_rectangles() {
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        cfg.rects.insert("infoSystem_0".into(), Rect { x: 80, y: 15, w: 55, h: 20 });
        cfg.rects.insert("infoBuildSystem_0".into(), Rect { x: 44, y: 52, w: 50, h: 20 });
        cfg.rects.insert("infoSystem_1".into(), Rect { x: 0, y: 0, w: 0, h: 0 });
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let origin = Vec2::new(100.0, 120.0);
        let icon = Some(R::new(20.0, 30.0, 20.0, 20.0));
        assert_eq!(system_readout_rect(&layout, origin, false, 0, icon), Some(R::new(180.0, 135.0, 55.0, 20.0)));
        assert_eq!(system_readout_rect(&layout, origin, true, 0, icon), Some(R::new(144.0, 172.0, 50.0, 20.0)));
        assert_eq!(system_readout_rect(&layout, origin, false, 1, icon), None);
        assert_eq!(system_readout_rect(&layout, origin, false, 2, icon), Some(R::new(42.0, 30.0, 40.0, 20.0)));
    }

    /// UI-28: each wireframe layer takes its system's state; a destroyed system's layer is pure
    /// red, the others white; hull damage plays no part.
    #[test]
    fn wireframe_layers_follow_their_systems_state() {
        use sta_sim::combat::damage::SystemView;
        let f = Fixed::from_f64;
        // Sim order: engines destroyed, weapons whole, shields 75 %, life support off, sensors whole.
        let view = SystemView { entity: EntityId(5), health: [Fixed::ZERO, Fixed::ONE, f(0.75), Fixed::ONE, Fixed::ONE], on: [true, true, true, false, true] };
        let feed = crate::ui::hud::status::StatusFeed { systems: vec![view], ..Default::default() };
        let at = |layer| layer_tint(Some(&feed), WireLayer { unit: EntityId(5), layer });
        assert_eq!(at(2), Color::srgb(1.0, 0.0, 0.0), "w2 engines: destroyed");
        assert_eq!(at(1), Color::WHITE, "w1 weapons: whole");
        assert_eq!(at(4), Color::srgb(0.25, 0.15, 0.15), "w4 shield generator: a quarter of its damage");
        assert_eq!(at(3), Color::srgb(1.0, 1.0, 0.0), "w3 life support: off");
        assert_eq!(at(5), Color::WHITE);
        assert_eq!(layer_tint(Some(&feed), WireLayer { unit: EntityId(9), layer: 2 }), Color::WHITE, "an unlisted unit is whole");
    }

    /// A bar with a cfg sprite: the picture tinted by the fill colour inside a clip the update
    /// resizes, over the track colour.
    #[test]
    fn a_sprite_bar_tints_its_picture_and_clips_it_at_the_value() {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let mut queue = CommandQueue::default();
        let track = Color::srgb(0.2, 0.2, 0.2);
        let blue = Color::srgb(0.0, 0.0, 1.0);
        sprite_bar(&mut Commands::new(&mut queue, &world), root, R::new(0.0, 0.0, 200.0, 10.0), 0.5, (track, blue), StockBar::Node(200.0), "large_dilithium_bar");
        queue.apply(&mut world);
        let outer = world.get::<Children>(root).unwrap()[0];
        assert_eq!(world.get::<BackgroundColor>(outer).unwrap().0, track);
        let clip = world.get::<Children>(outer).unwrap()[0];
        assert_eq!(world.get::<Node>(clip).unwrap().width, Val::Px(100.0));
        let picture = world.get::<Children>(clip).unwrap()[0];
        let image = world.get::<GuiImage>(picture).unwrap();
        assert_eq!((image.names[0].as_str(), image.color), ("large_dilithium_bar", blue));
        assert_eq!(world.get::<Node>(picture).unwrap().width, Val::Px(200.0), "the picture keeps the full bar's size");
    }


    /// The named sprite takes the author's fill when present and the fallback otherwise.
    #[test]
    fn cfg_bars_tint_named_sprites_and_respect_authored_colours() {
        use sta_content::gui_cfg::{Colour, GuiLayout};
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        cfg.names.insert("infoSingleHullBar".into(), "large_hull_bar".into());
        cfg.names.insert("infoSingleShieldBar".into(), "large_shield_bar".into());
        cfg.colours.insert("infoSingleShieldBarColor".into(), Colour { r: 0.0, g: 0.0, b: 1.0, a: 1.0 });
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let mut queue = CommandQueue::default();
        let r = R::new(0.0, 0.0, 100.0, 10.0);
        let green = Color::srgb(0.0, 1.0, 0.0);
        {
            let mut c = Commands::new(&mut queue, &world);
            cfg_bar(&mut c, root, &layout, r, 1.0, (&[], &["infoSingleHullBarColor"], green), StockBar::Hull(100.0), "infoSingleHullBar");
            cfg_bar(&mut c, root, &layout, r, 1.0, (&[], &["infoSingleShieldBarColor"], green), StockBar::Shield(100.0), "infoSingleShieldBar");
            cfg_bar(&mut c, root, &layout, r, 1.0, (&[], &[], green), StockBar::Progress(100.0), "infoSingleConstructionBar");
        }
        queue.apply(&mut world);
        let images: Vec<(String, Color)> = world.query::<&GuiImage>().iter(&world).map(|i| (i.names[0].clone(), i.color)).collect();
        assert!(images.contains(&("large_hull_bar".to_owned(), green)), "{images:?}");
        assert!(images.contains(&("large_shield_bar".to_owned(), Color::srgba(0.0, 0.0, 1.0, 1.0))), "{images:?}");
        assert_eq!(images.len(), 2, "no sprite named: the construction bar stays flat");
    }

    /// UI-48: a group tile carries its shield share; a bar is drawn at `infoMultiShipShield_<n>`.
    #[test]
    fn group_tiles_draw_their_shield_bar_from_the_cfg() {
        use sta_content::gui_cfg::{GuiLayout, Rect};
        let mut cfg = GuiLayout { canvas: (1600, 1200), ..Default::default() };
        cfg.rects.insert("infoMultiShipShield_0".into(), Rect { x: 10, y: 20, w: 72, h: 7 });
        // A tile is drawn only where its icon rectangle is authored.
        cfg.rects.insert("infoMultiShipIcon_0".into(), Rect { x: 10, y: 30, w: 72, h: 40 });
        cfg.rects.insert("infoMultiShipIcon_1".into(), Rect { x: 90, y: 30, w: 72, h: 40 });
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let mut queue = CommandQueue::default();
        let feed = HudFeed::default();
        let theme = Theme::default();
        let tiles = vec![model::Tile { id: EntityId(1), name: "A".into(), hull: 1.0, shield: Some(0.5) }, model::Tile { id: EntityId(2), name: "B".into(), hull: 1.0, shield: None }];
        spawn_group(&mut Commands::new(&mut queue, &world), root, &layout, &feed, &theme, Vec2::ZERO, 1, &tiles);
        queue.apply(&mut world);
        let shields: Vec<(EntityId, Val)> = world.query::<(&StockBar, &Node)>().iter(&world).filter_map(|(k, n)| match k { StockBar::GroupShield(id, _) => Some((*id, n.width)), _ => None }).collect();
        assert_eq!(shields, [(EntityId(1), Val::Px(36.0))], "half of 72 for the first tile; none without the key or shields");
    }
}
