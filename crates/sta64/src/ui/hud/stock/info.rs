//! Stock resource and selection display. Only values exposed by `HudFeed` are drawn.

use bevy::picking::Pickable;
use bevy::prelude::*;
use sta_sim::{EntityId, Fixed};

use super::super::layout::R;
use super::super::model::{self, HudFeed, Info};
use super::super::panels::GroupTile;
use super::layout::{StockLayout, info_variant};
use crate::ui::gui_sprites::{GuiImage, GuiImageSampling};
use crate::ui::notices::Tooltip;
use crate::ui::sprite_font::{SpriteText, sprite_text};
use crate::ui::theme::{Theme, UiButton, WidgetState};

#[path = "identity.rs"]
mod identity;

#[cfg(test)]
pub(in crate::ui::hud) use identity::{IdentityField, IdentityText};

#[derive(Component, Clone, Copy)]
pub(in crate::ui::hud) enum StockText {
    Resource(usize),
}

#[derive(Component, Clone, Copy)]
pub(in crate::ui::hud) enum StockBar {
    Hull(f32),
    Shield(f32),
    Progress(f32),
    Group(EntityId, f32),
}

fn node(r: R) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(r.x),
        top: Val::Px(r.y),
        width: Val::Px(r.w),
        height: Val::Px(r.h),
        ..default()
    }
}

fn place(origin: Vec2, r: R) -> R {
    R::new(origin.x + r.x, origin.y + r.y, r.w, r.h)
}

fn picture(commands: &mut Commands, root: Entity, r: R, name: &str, frame: usize) {
    commands
        .entity(root)
        .with_child((node(r), Pickable::IGNORE, GuiImage::new(name).with_frame(frame)));
}

fn text(commands: &mut Commands, root: Entity, r: R, value: String, color: Color, size: f32, kind: Option<StockText>) -> Entity {
    // SpriteFontPlugin replaces the SpriteText entity's Node while resolving glyphs.
    // Keep authored placement on a separate parent that it never rewrites.
    let wrapper = commands.spawn((node(r), Pickable::IGNORE)).id();
    let mut e = commands.spawn((Node::default(), sprite_text(value, size.max(18.0), color)));
    if let Some(kind) = kind {
        e.insert(kind);
    }
    let child = e.id();
    commands.entity(wrapper).add_child(child);
    commands.entity(root).add_child(wrapper);
    child
}

fn bar(commands: &mut Commands, root: Entity, r: R, value: f32, color: Color, kind: StockBar) {
    commands.entity(root).with_child((
        node(r),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
        Pickable::IGNORE,
        children![(
            kind,
            Node {
                width: Val::Px(r.w * value.clamp(0.0, 1.0)),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(color),
            Pickable::IGNORE,
        )],
    ));
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

fn wireframe_rect(layout: &StockLayout, origin: Vec2, builder: bool) -> Option<R> {
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
fn wireframe_layers(commands: &mut Commands, root: Entity, r: R, model: &str) {
    let stem = model_stem(model);
    if stem.is_empty() {
        return;
    }
    for layer in 1..=5 {
        let mut image = GuiImage::new("").with_sampling(GuiImageSampling::Linear);
        image.names = wireframe_names(&stem, layer);
        commands.entity(root).with_child((node(r), Pickable::IGNORE, image));
    }
}

fn spawn_resource(commands: &mut Commands, root: Entity, layout: &StockLayout, feed: &HudFeed, theme: &Theme) {
    let Some(area) = layout.panel("resource") else {
        return;
    };
    let origin = Vec2::new(area.x, area.y);
    for piece in layout.pieces("resourcePanel") {
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
        text(
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
    if let Some(r) = chosen_rect(
        layout,
        origin,
        single.builder,
        "infoBuildName",
        "infoSingleNameTextArea",
    ) {
        text(
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
        && let Some(unit) = feed.view.unit(single.id)
        && let Some(def) = feed.content.as_deref().and_then(|db| db.unit(unit.def))
    {
        wireframe_layers(commands, root, r, &def.model);
    }
    if let Some(r) = chosen_rect(
        layout,
        origin,
        single.builder,
        "infoBuildHullBar",
        "infoSingleHullBarArea",
    ) {
        bar(commands, root, r, single.hull, theme.good, StockBar::Hull(r.w));
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
        bar(commands, root, r, shield, theme.accent2, StockBar::Shield(r.w));
    }
    if let Some(progress) = single.progress
        && let Some(r) = layout.at(origin, "infoSingleConstructionBarArea")
    {
        bar(commands, root, r, progress, theme.warn, StockBar::Progress(r.w));
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
                BackgroundColor(theme.button),
            ))
            .id();
        commands.entity(root).add_child(entity);
        if let Some(unit) = feed.view.unit(tile.id)
            && let Some(def) = feed.content.as_deref().and_then(|db| db.unit(unit.def))
        {
            wireframe_layers(commands, entity, R::new(0.0, 0.0, r.w, r.h), &def.model);
        }
        text(
            commands,
            entity,
            R::new(2.0, 2.0, r.w - 4.0, r.h - 4.0),
            tile.name.chars().take(8).collect(),
            theme.text,
            theme.micro_size,
            None,
        );
        let hull_key = if small {
            format!("infoMultiShipHullSm_{i}")
        } else {
            format!("infoMultiShipHull_{i}")
        };
        if let Some(hull) = layout.at(origin, &hull_key) {
            bar(
                commands,
                root,
                hull,
                tile.hull,
                theme.good,
                StockBar::Group(tile.id, hull.w),
            );
        }
    }
}

pub(super) fn spawn(commands: &mut Commands, root: Entity, layout: &StockLayout, feed: &HudFeed, theme: &Theme) {
    spawn_resource(commands, root, layout, feed, theme);
    let info = model::info(feed);
    let variant = info_variant(&info);
    let Some(area) = layout.info_area(variant) else {
        return;
    };
    let origin = Vec2::new(area.x, area.y);
    for piece in layout.pieces(info_background(variant)) {
        picture(commands, root, place(origin, piece.rect), &piece.name, piece.frame);
    }
    match &info {
        Info::None => {}
        Info::Single(single) => spawn_single(commands, root, layout, feed, theme, origin, single),
        Info::Group(tiles) => spawn_group(commands, root, layout, feed, theme, origin, variant, tiles),
    }
}

pub(in crate::ui::hud) fn update(
    feed: Res<HudFeed>,
    mut texts: Query<(&StockText, &mut SpriteText)>,
    mut bars: Query<(&StockBar, &mut Node)>,
) {
    let info = model::info(&feed);
    for (kind, mut text) in &mut texts {
        let StockText::Resource(id) = kind;
        let value = resource_text(&feed, *id).unwrap_or_default();
        if text.text != value {
            text.text = value;
        }
    }
    for (kind, mut node) in &mut bars {
        let (value, width) = match (kind, &info) {
            (StockBar::Hull(w), Info::Single(s)) => (s.hull, *w),
            (StockBar::Shield(w), Info::Single(s)) => (s.shield.unwrap_or(0.0), *w),
            (StockBar::Progress(w), Info::Single(s)) => (s.progress.unwrap_or(0.0), *w),
            (StockBar::Group(id, w), Info::Group(tiles)) => (
                tiles.iter().find(|tile| tile.id == *id).map_or(0.0, |tile| tile.hull),
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
    use sta_sim::{
        Fixed, PlayerId, TeamId,
        view::{PlayerView, WorldView},
    };
    use std::sync::Arc;

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
}
