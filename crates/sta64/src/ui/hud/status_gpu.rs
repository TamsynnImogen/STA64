//! Status widgets over the existing stock or LCARS information panel.
//! This owns its descendants and never alters the stock info/resource trees.

use bevy::picking::Pickable;
use bevy::prelude::*;
use sta_sim::{EntityId, Fixed};

use super::layout::R;
use super::model::{self, HudFeed, Info};
use super::status::{self, JobState, QueueItem, SelectedStatus, StatusFeed};
use super::stock::{self, layout::StockLayout};
use super::{HudConfig, HudLayoutRes, HudRoot};
use crate::ui::gui_sprites::{GuiImage, GuiImageSampling};
use crate::ui::profile::UiProfile;
use crate::ui::sprite_font::{SpriteText, SpriteTextStyle, sprite_text_styled};
use crate::ui::theme::Theme;

#[derive(Component)]
pub(super) struct StatusRoot;

#[derive(Component, Clone, Copy)]
pub(super) enum LiveValue {
    QueueProgress(usize, f32),
    Construction(f32),
    ConstructionPercent,
    Cargo(f32),
    CargoAmount,
}

#[derive(Clone, Debug, PartialEq)]
struct Shape {
    selected: EntityId,
    stock: bool,
    queue: Vec<QueueShape>,
    cargo: bool,
    cargo_resource: Option<u8>,
    construction: bool,
    slots: Vec<(usize, R)>,
    progress: Option<R>,
    cargo_bar: Option<R>,
    lcars_card: Option<R>,
}

#[derive(Clone, Debug, PartialEq)]
struct QueueShape {
    def: sta_sim::ContentKey,
    state: JobState,
    name: String,
    model: Option<String>,
    odf: Option<String>,
}

#[cfg(test)]
#[path = "status_frames.rs"]
mod frames;

#[derive(Resource, Default)]
pub(super) struct StatusUiState {
    last: Option<Shape>,
}

impl StatusUiState {
    pub(super) fn fallback_card(&self) -> Option<R> {
        self.last.as_ref().and_then(|shape| shape.lcars_card)
    }
}

fn rect(r: R) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(r.x),
        top: Val::Px(r.y),
        width: Val::Px(r.w),
        height: Val::Px(r.h),
        ..default()
    }
}

#[allow(clippy::too_many_arguments)]
fn text(
    commands: &mut Commands,
    parent: Entity,
    r: R,
    value: String,
    theme: &Theme,
    live: Option<LiveValue>,
    stock: bool,
) {
    let style = SpriteTextStyle::default().centered().wrapped(r.w);
    let style = if stock { style.stock_label() } else { style };
    let wrapper = commands
        .spawn((rect(r), Pickable::IGNORE, ChildOf(parent)))
        .id();
    let mut child = commands.spawn((
        sprite_text_styled(value, theme.micro_size.max(16.0), theme.text, style),
        ChildOf(wrapper),
    ));
    if let Some(live) = live {
        child.insert(live);
    }
}

#[allow(clippy::too_many_arguments)]
fn clipped_bar(
    commands: &mut Commands,
    parent: Entity,
    r: R,
    fraction: f32,
    theme: &Theme,
    sprite: Option<&str>,
    tint: Color,
    kind: LiveValue,
) {
    let track = commands
        .spawn((
            rect(r),
            BackgroundColor(theme.track),
            Pickable::IGNORE,
            ChildOf(parent),
        ))
        .id();
    let clip = commands
        .spawn((
            kind,
            Node {
                width: Val::Px(r.w * fraction.clamp(0.0, 1.0)),
                height: Val::Percent(100.0),
                overflow: Overflow::clip(),
                ..default()
            },
            Pickable::IGNORE,
            ChildOf(track),
        ))
        .id();
    let fill = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(r.w),
                height: Val::Px(r.h),
                ..default()
            },
            BackgroundColor(tint),
            Pickable::IGNORE,
            ChildOf(clip),
        ))
        .id();
    if let Some(sprite) = sprite.filter(|s| !s.is_empty()) {
        commands.entity(fill).insert(
            GuiImage::new(sprite)
                .with_color(tint)
                .with_sampling(GuiImageSampling::Linear),
        );
    }
}

fn sprite_candidates(item: &QueueItem) -> Vec<String> {
    let mut names = Vec::new();
    for source in [item.model.as_deref(), item.odf.as_deref()]
        .into_iter()
        .flatten()
    {
        let basename = source.rsplit(['/', '\\']).next().unwrap_or(source).trim();
        let stem = match basename.rsplit_once('.') {
            Some((stem, ext))
                if ["sod", "odf", "glb", "gltf", "m3d"]
                    .iter()
                    .any(|known| ext.eq_ignore_ascii_case(known)) =>
            {
                stem
            }
            _ => basename,
        };
        if !stem.is_empty() {
            let name = format!("b_{stem}");
            if !names
                .iter()
                .any(|old: &String| old.eq_ignore_ascii_case(&name))
            {
                names.push(name);
            }
        }
    }
    names
}

fn stock_shape(
    selected: EntityId,
    status: &SelectedStatus,
    layout: &StockLayout,
    info: &Info,
) -> Shape {
    let variant = stock::layout::info_variant(info);
    let origin = layout.info_origin(variant);
    let slots: Vec<_> = (0..10)
        .filter_map(|i| {
            layout
                .at(origin, &format!("infoBuildQueue_{i}"))
                .map(|r| (i, r))
        })
        .collect();
    let progress = layout.at(origin, "infoProgressBar");
    let cargo_bar = layout.at(origin, "infoSingleDilithiumBarArea");
    let fallback_queue = slots.is_empty() && !status.queue.is_empty();
    let fallback_progress = !slots.is_empty()
        && progress.is_none()
        && status.queue.first().and_then(|job| job.progress).is_some();
    let fallback_construction = status.construction.is_some()
        && layout.at(origin, "infoSingleConstructionBarArea").is_none()
        && layout.at(origin, "infoSingleConstructionArea").is_none();
    let fallback_cargo = status.cargo.is_some() && cargo_bar.is_none();
    // Some stock sheets, notably A1, omit queue slots. Other sheets may omit
    // individual bars. Only the missing widgets use the adjacent card.
    let lcars_card =
        if fallback_queue || fallback_progress || fallback_construction || fallback_cargo {
            layout.info_area(variant).map(|panel| {
                let rows = if fallback_queue {
                    status.queue.len().min(10)
                } else if fallback_progress {
                    1
                } else {
                    0
                } + usize::from(fallback_cargo)
                    + usize::from(fallback_construction);
                let h = rows as f32 * 26.0 + 12.0;
                R::new(panel.x, (panel.y - h - 8.0).max(0.0), panel.w, h)
            })
        } else {
            None
        };
    Shape {
        selected,
        stock: true,
        queue: status
            .queue
            .iter()
            .map(|q| QueueShape {
                def: q.def,
                state: q.state,
                name: q.name.clone(),
                model: q.model.clone(),
                odf: q.odf.clone(),
            })
            .collect(),
        cargo: status.cargo.is_some(),
        cargo_resource: status.cargo.and_then(|c| c.resource),
        construction: status.construction.is_some(),
        slots,
        progress,
        cargo_bar,
        lcars_card,
    }
}

fn lcars_shape(
    selected: EntityId,
    status: &SelectedStatus,
    layout: &HudLayoutRes,
    info: &Info,
    theme: &Theme,
) -> Shape {
    let panel = layout.0.bottom_row(info.height()).info;
    let rows = status.queue.len().min(10)
        + usize::from(status.cargo.is_some())
        + usize::from(status.construction.is_some());
    let height = (rows as f32 * 26.0 + 12.0).max(0.0);
    let card = (rows > 0).then(|| {
        R::new(
            panel.x,
            (panel.y - height - theme.gap).max(0.0),
            panel.w,
            height,
        )
    });
    Shape {
        selected,
        stock: false,
        queue: status
            .queue
            .iter()
            .map(|q| QueueShape {
                def: q.def,
                state: q.state,
                name: q.name.clone(),
                model: q.model.clone(),
                odf: q.odf.clone(),
            })
            .collect(),
        cargo: status.cargo.is_some(),
        cargo_resource: status.cargo.and_then(|c| c.resource),
        construction: status.construction.is_some(),
        slots: Vec::new(),
        progress: None,
        cargo_bar: None,
        lcars_card: card,
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_stock(
    commands: &mut Commands,
    root: Entity,
    feed: &HudFeed,
    status: &SelectedStatus,
    shape: &Shape,
    layout: &StockLayout,
    theme: &Theme,
    info: &Info,
) {
    if shape.lcars_card.is_some() {
        let origin = layout.info_origin(stock::layout::info_variant(info));
        let fallback = SelectedStatus {
            queue: if shape.slots.is_empty() {
                status.queue.clone()
            } else if shape.progress.is_none() {
                status
                    .queue
                    .first()
                    .filter(|job| job.progress.is_some())
                    .cloned()
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            },
            construction: status.construction.filter(|_| {
                layout.at(origin, "infoSingleConstructionBarArea").is_none()
                    && layout.at(origin, "infoSingleConstructionArea").is_none()
            }),
            cargo: status.cargo.filter(|_| shape.cargo_bar.is_none()),
        };
        draw_lcars(commands, root, feed, &fallback, shape, theme);
    }
    for (item, (slot_index, r)) in status.queue.iter().zip(&shape.slots) {
        let slot = commands
            .spawn((rect(*r), Pickable::IGNORE, ChildOf(root)))
            .id();
        if let Some(sprite) = layout.name(&format!("infoBuildQueueSlot_{slot_index}")) {
            commands.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                GuiImage::new(sprite),
                Pickable::IGNORE,
                ChildOf(slot),
            ));
        }
        let icon = sprite_candidates(item);
        if !icon.is_empty() {
            let mut image = GuiImage::new(icon[0].clone());
            image.names = icon;
            commands.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(2.0),
                    top: Val::Px(2.0),
                    width: Val::Px((r.w - 4.0).max(0.0)),
                    height: Val::Px((r.h - 20.0).max(0.0)),
                    ..default()
                },
                image,
                Pickable::IGNORE,
                ChildOf(slot),
            ));
        }
        let label = format!(
            "{} {}",
            item.name,
            match item.state {
                JobState::Building => "▶",
                JobState::AwaitingResources => "WAIT",
                JobState::Queued => "…",
            }
        );
        text(
            commands,
            slot,
            R::new(2.0, (r.h - 22.0).max(0.0), (r.w - 4.0).max(0.0), 20.0),
            label,
            theme,
            None,
            true,
        );
    }
    if status.queue.len() > shape.slots.len()
        && let Some((_, last)) = shape.slots.last()
    {
        text(
            commands,
            root,
            R::new(last.x + last.w - 35.0, last.y, 35.0, 20.0),
            format!("+{}", status.queue.len() - shape.slots.len()),
            theme,
            None,
            true,
        );
    }
    if let Some(head) = status.queue.first()
        && let Some(value) = head.progress
        && let Some(r) = shape.progress
    {
        clipped_bar(
            commands,
            root,
            r,
            value,
            theme,
            layout.name("infoProgressBar"),
            theme.warn,
            LiveValue::QueueProgress(0, r.w),
        );
    }
    let origin = layout.info_origin(stock::layout::info_variant(info));
    if let Some(value) = status.construction
        && layout.at(origin, "infoSingleConstructionBarArea").is_none()
        && let Some(r) = layout.at(origin, "infoSingleConstructionArea")
    {
        clipped_bar(
            commands,
            root,
            r,
            value,
            theme,
            layout.name("infoSingleConstructionBar"),
            layout
                .colour("infoSingleConstructionBarColor")
                .unwrap_or(theme.warn),
            LiveValue::Construction(r.w),
        );
    }
    if let Some(cargo) = status.cargo
        && let Some(r) = shape.cargo_bar
    {
        clipped_bar(
            commands,
            root,
            r,
            cargo.fraction,
            theme,
            layout.name("infoSingleDilithiumBar"),
            cargo_colour(layout, cargo.resource, theme),
            LiveValue::Cargo(r.w),
        );
        text(
            commands,
            root,
            R::new(r.x, r.y, r.w, r.h),
            cargo_text(feed, cargo.amount, cargo.capacity, cargo.resource),
            theme,
            Some(LiveValue::CargoAmount),
            true,
        );
    }
}

fn cargo_text(feed: &HudFeed, amount: Fixed, capacity: Fixed, resource: Option<u8>) -> String {
    let name = resource
        .and_then(|id| {
            feed.content
                .as_deref()
                .map(|db| db.resources().name(usize::from(id)))
        })
        .unwrap_or("CARGO");
    format!(
        "{} {:.0}/{:.0}",
        name.to_uppercase(),
        amount.to_f32(),
        capacity.to_f32()
    )
}

fn cargo_colour(layout: &StockLayout, resource: Option<u8>, theme: &Theme) -> Color {
    let key = match resource {
        Some(1) => "infoSingleLatinumBarColor",
        Some(2) => "infoSingleMetalBarColor",
        Some(3) => "infoSingleBiomatterBarColor",
        _ => "infoSingleDilithiumBarColor",
    };
    layout
        .colour(key)
        .or_else(|| {
            resource
                .and_then(|id| layout.colour(&format!("resource_{}_color", usize::from(id) + 2)))
        })
        .unwrap_or(theme.accent3)
}

fn draw_lcars(
    commands: &mut Commands,
    root: Entity,
    feed: &HudFeed,
    status: &SelectedStatus,
    shape: &Shape,
    theme: &Theme,
) {
    let Some(card) = shape.lcars_card else { return };
    commands.spawn((
        rect(card),
        BackgroundColor(theme.panel.with_alpha(0.92)),
        BorderColor::all(theme.accent2),
        Pickable::IGNORE,
        ChildOf(root),
    ));
    let mut y = card.y + 5.0;
    for (i, item) in status.queue.iter().take(10).enumerate() {
        let label = match item.state {
            JobState::Building => "BUILDING",
            JobState::AwaitingResources => "WAITING",
            JobState::Queued => "QUEUED",
        };
        let row = R::new(card.x + 8.0, y, card.w - 16.0, 22.0);
        text(
            commands,
            root,
            row,
            format!("{}  {label}", item.name),
            theme,
            None,
            false,
        );
        if let Some(value) = item.progress {
            let bar = R::new(card.x + card.w - 100.0, y + 18.0, 90.0, 3.0);
            clipped_bar(
                commands,
                root,
                bar,
                value,
                theme,
                None,
                theme.warn,
                LiveValue::QueueProgress(i, bar.w),
            );
        }
        y += 26.0;
    }
    if status.queue.len() > 10 {
        text(
            commands,
            root,
            R::new(card.x + card.w - 55.0, card.y + 3.0, 50.0, 20.0),
            format!("+{}", status.queue.len() - 10),
            theme,
            None,
            false,
        );
    }
    if let Some(value) = status.construction {
        text(
            commands,
            root,
            R::new(card.x + 8.0, y, (card.w - 118.0).max(30.0), 22.0),
            format!("CONSTRUCTION {}%", (value * 100.0).round() as i32),
            theme,
            Some(LiveValue::ConstructionPercent),
            false,
        );
        let bar = R::new(card.x + card.w - 100.0, y + 18.0, 90.0, 3.0);
        clipped_bar(
            commands,
            root,
            bar,
            value,
            theme,
            None,
            theme.warn,
            LiveValue::Construction(bar.w),
        );
        y += 26.0;
    }
    if let Some(cargo) = status.cargo {
        text(
            commands,
            root,
            R::new(card.x + 8.0, y, card.w - 16.0, 22.0),
            cargo_text(feed, cargo.amount, cargo.capacity, cargo.resource),
            theme,
            Some(LiveValue::CargoAmount),
            false,
        );
        let bar = R::new(card.x + card.w - 100.0, y + 18.0, 90.0, 3.0);
        clipped_bar(
            commands,
            root,
            bar,
            cargo.fraction,
            theme,
            None,
            theme.accent3,
            LiveValue::Cargo(bar.w),
        );
    }
}

/// Rebuild only when selection, authored geometry, queue identity, or theme changes.
#[allow(clippy::too_many_arguments)]
pub(super) fn refresh(
    mut commands: Commands,
    feed: Res<HudFeed>,
    source: Res<StatusFeed>,
    config: Res<HudConfig>,
    theme: Res<Theme>,
    profile: Option<Res<UiProfile>>,
    layout: Res<HudLayoutRes>,
    stock_state: Res<stock::StockState>,
    roots: Query<Entity, With<HudRoot>>,
    existing: Query<Entity, With<StatusRoot>>,
    mut state: ResMut<StatusUiState>,
) {
    let Ok(root) = roots.single() else {
        state.last = None;
        return;
    };
    let status = status::selected(&feed, &source);
    let Info::Single(info) = model::info(&feed) else {
        for entity in &existing {
            commands.entity(entity).despawn();
        }
        state.last = None;
        return;
    };
    let stock = stock::enabled(profile.as_deref(), &config);
    let shape = if stock {
        let Some(stock_layout) = stock_state.layout.as_ref() else {
            for entity in &existing {
                commands.entity(entity).despawn();
            }
            state.last = None;
            return;
        };
        stock_shape(info.id, &status, stock_layout, &Info::Single(info.clone()))
    } else {
        lcars_shape(
            info.id,
            &status,
            &layout,
            &Info::Single(info.clone()),
            &theme,
        )
    };
    if status.queue.is_empty() && status.cargo.is_none() && status.construction.is_none() {
        for entity in &existing {
            commands.entity(entity).despawn();
        }
        state.last = None;
        return;
    }
    if state.last.as_ref() == Some(&shape)
        && !theme.is_changed()
        && !config.is_changed()
        && !layout.is_changed()
        && !existing.is_empty()
    {
        return;
    }
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let overlay = commands
        .spawn((
            StatusRoot,
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            ChildOf(root),
        ))
        .id();
    if stock {
        if let Some(stock_layout) = stock_state.layout.as_ref() {
            draw_stock(
                &mut commands,
                overlay,
                &feed,
                &status,
                &shape,
                stock_layout,
                &theme,
                &Info::Single(info),
            );
        }
    } else {
        draw_lcars(&mut commands, overlay, &feed, &status, &shape, &theme);
    }
    state.last = Some(shape);
}

/// Numeric values change each tick without disturbing sprite/glyph owners.
pub(super) fn update_values(
    feed: Res<HudFeed>,
    source: Res<StatusFeed>,
    mut bars: Query<(&LiveValue, &mut Node)>,
    mut texts: Query<(&LiveValue, &mut SpriteText)>,
) {
    let status = status::selected(&feed, &source);
    for (kind, mut node) in &mut bars {
        let value = match kind {
            LiveValue::QueueProgress(i, width) => status
                .queue
                .get(*i)
                .and_then(|q| q.progress)
                .map(|p| p * width),
            LiveValue::Construction(width) => status.construction.map(|p| p * width),
            LiveValue::Cargo(width) => status.cargo.map(|c| c.fraction * width),
            _ => None,
        };
        if let Some(width) = value {
            let width = Val::Px(width.clamp(0.0, f32::MAX));
            if node.width != width {
                node.width = width;
            }
        }
    }
    for (kind, mut text) in &mut texts {
        let value = match kind {
            LiveValue::ConstructionPercent => status
                .construction
                .map(|p| format!("CONSTRUCTION {}%", (p * 100.0).round() as i32)),
            LiveValue::CargoAmount => status
                .cargo
                .map(|c| cargo_text(&feed, c.amount, c.capacity, c.resource)),
            _ => None,
        };
        if let Some(value) = value
            && text.text != value
        {
            text.text = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use sta_content::gui_cfg::{GuiLayout, Panel, Rect};
    use sta_sim::ContentKey;

    use super::*;
    use crate::ui::hud::model::Single;

    fn info(builder: bool) -> Info {
        Info::Single(Single {
            id: EntityId(7),
            name: "Yard".into(),
            captain: None,
            registry: None,
            class: "Station".into(),
            own: true,
            hull: 1.0,
            hull_now: 100,
            hull_max: 100,
            shield: None,
            progress: None,
            builder,
        })
    }

    fn queued() -> SelectedStatus {
        SelectedStatus {
            queue: vec![
                QueueItem {
                    def: ContentKey(1),
                    odf: Some("ship.odf".into()),
                    model: Some("models/ship.sod".into()),
                    name: "Ship".into(),
                    state: JobState::Building,
                    progress: Some(0.5),
                },
                QueueItem {
                    def: ContentKey(2),
                    odf: Some("frigate".into()),
                    model: None,
                    name: "Frigate".into(),
                    state: JobState::Queued,
                    progress: None,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn queue_uses_only_authored_stock_slots_and_preserves_overflow() {
        let mut cfg = GuiLayout {
            canvas: (1600, 1200),
            ..Default::default()
        };
        cfg.rects.insert(
            "infoBuildQueue_0".into(),
            Rect {
                x: 10,
                y: 10,
                w: 96,
                h: 96,
            },
        );
        cfg.rects.insert(
            "infoBuildQueue_1".into(),
            Rect {
                x: 110,
                y: 10,
                w: 0,
                h: 96,
            },
        );
        cfg.rects.insert(
            "infoProgressBar".into(),
            Rect {
                x: 10,
                y: 110,
                w: 200,
                h: 12,
            },
        );
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let shape = stock_shape(EntityId(7), &queued(), &layout, &info(true));
        assert_eq!(shape.slots.len(), 1);
        assert_eq!(shape.queue.len(), 2); // caller renders the extra-count badge
        assert!(shape.progress.is_some());
        assert_eq!(shape.lcars_card, None);
    }

    #[test]
    fn absent_stock_slots_and_lcars_use_readable_fallback_cards() {
        let mut cfg = GuiLayout {
            canvas: (1600, 1200),
            ..Default::default()
        };
        cfg.panels.insert(
            "info".into(),
            Panel {
                name: "info".into(),
                area: Some(Rect {
                    x: 500,
                    y: 950,
                    w: 400,
                    h: 200,
                }),
                ..Default::default()
            },
        );
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        assert!(
            stock_shape(EntityId(7), &queued(), &layout, &info(true))
                .lcars_card
                .is_some()
        );
        let lcars = lcars_shape(
            EntityId(7),
            &queued(),
            &HudLayoutRes(super::super::layout::HudLayout::new(None, 16.0 / 9.0)),
            &info(true),
            &Theme::default(),
        );
        assert!(lcars.lcars_card.is_some());
    }

    #[test]
    fn missing_native_cargo_and_construction_bars_use_fallback_with_slots_present() {
        let mut cfg = GuiLayout {
            canvas: (1600, 1200),
            ..Default::default()
        };
        cfg.panels.insert(
            "info".into(),
            Panel {
                name: "info".into(),
                area: Some(Rect {
                    x: 500,
                    y: 950,
                    w: 400,
                    h: 200,
                }),
                ..Default::default()
            },
        );
        cfg.rects.insert(
            "infoBuildQueue_0".into(),
            Rect {
                x: 10,
                y: 10,
                w: 96,
                h: 96,
            },
        );
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let mut status = queued();
        status.construction = Some(0.3);
        status.cargo = Some(super::super::status::CargoStatus {
            amount: Fixed::ZERO,
            capacity: Fixed::from_int(150),
            resource: None,
            fraction: 0.0,
        });
        let shape = stock_shape(EntityId(7), &status, &layout, &info(true));
        assert_eq!(shape.slots.len(), 1);
        assert!(shape.cargo_bar.is_none());
        assert!(shape.lcars_card.is_some());
    }

    #[test]
    fn active_queue_progress_falls_back_when_stock_has_slots_but_no_bar() {
        let mut cfg = GuiLayout {
            canvas: (1600, 1200),
            ..Default::default()
        };
        cfg.panels.insert(
            "info".into(),
            Panel {
                name: "info".into(),
                area: Some(Rect {
                    x: 500,
                    y: 950,
                    w: 400,
                    h: 200,
                }),
                ..Default::default()
            },
        );
        cfg.rects.insert(
            "infoBuildQueue_0".into(),
            Rect {
                x: 10,
                y: 10,
                w: 96,
                h: 96,
            },
        );
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        assert!(
            stock_shape(EntityId(7), &queued(), &layout, &info(true))
                .lcars_card
                .is_some()
        );
        cfg.rects.insert(
            "infoProgressBar".into(),
            Rect {
                x: 10,
                y: 110,
                w: 200,
                h: 12,
            },
        );
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        assert!(
            stock_shape(EntityId(7), &queued(), &layout, &info(true))
                .lcars_card
                .is_none()
        );
    }

    #[test]
    fn queued_icon_uses_model_then_odf_names() {
        let mut item = queued().queue.remove(0);
        item.model = Some("models/hero.sod".into());
        assert_eq!(sprite_candidates(&item), ["b_hero", "b_ship"]);
    }

    #[test]
    fn mod_model_change_invalidates_queue_art_shape() {
        let cfg = GuiLayout {
            canvas: (1600, 1200),
            ..Default::default()
        };
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let before = queued();
        let mut after = before.clone();
        after.queue[0].model = Some("models/replacement.sod".into());
        assert_ne!(
            stock_shape(EntityId(7), &before, &layout, &info(true)),
            stock_shape(EntityId(7), &after, &layout, &info(true))
        );
    }

    #[test]
    fn headless_overlay_resets_on_selection_fog_and_theme_changes() {
        use sta_sim::view::UnitFlags;
        use sta_sim::{ContentKey, PlayerId, UnitView, Vec3, WorldView};
        use std::sync::Arc;

        let own = UnitView {
            id: EntityId(7),
            def: ContentKey::NONE,
            owner: PlayerId(1),
            pos: Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE, Fixed::ONE],
            shields: [[Fixed::ZERO; 2]; 4],
            flags: UnitFlags::SELECTABLE,
            progress: None,
            crew: None,
            energy: None,
            autonomy: None,
        };
        let mut enemy = own.clone();
        enemy.id = EntityId(8);
        enemy.owner = PlayerId(2);
        enemy.progress = Some(Fixed::from_f64(0.4));
        fn roots(app: &mut App) -> Vec<Entity> {
            let world = app.world_mut();
            let mut query = world.query_filtered::<Entity, With<StatusRoot>>();
            query.iter(world).collect()
        }
        let mut app = App::new();
        app.insert_resource(HudFeed {
            view: Arc::new(WorldView {
                units: vec![own, enemy],
                ..Default::default()
            }),
            player: PlayerId(1),
            selected: vec![EntityId(7)],
            content: None,
            names: Default::default(),
        });
        app.insert_resource(StatusFeed {
            cargo: vec![sta_sim::economy::view::CargoView {
                entity: EntityId(7),
                amount: Fixed::ZERO,
                capacity: Fixed::from_int(150),
                resource: None,
            }],
            ..Default::default()
        });
        app.insert_resource(HudConfig::default());
        app.insert_resource(HudLayoutRes(super::super::layout::HudLayout::new(
            None,
            16.0 / 9.0,
        )));
        app.insert_resource(Theme::default());
        app.insert_resource(stock::StockState::default());
        app.insert_resource(StatusUiState::default());
        app.world_mut().spawn((HudRoot, Node::default()));
        app.add_systems(Update, refresh);
        app.update();
        let first = roots(&mut app)[0];
        app.world_mut().resource_mut::<Theme>().accent = Color::WHITE;
        app.update();
        let second = roots(&mut app)[0];
        assert_ne!(first, second);
        app.world_mut().resource_mut::<HudFeed>().selected = vec![EntityId(7), EntityId(8)];
        app.update();
        assert!(roots(&mut app).is_empty());
        app.world_mut().resource_mut::<HudFeed>().selected = vec![EntityId(8)];
        app.update();
        assert_eq!(roots(&mut app).len(), 1);
        let mut hidden = (*app.world().resource::<HudFeed>().view).clone();
        hidden.visibility = vec![(PlayerId(1), vec![EntityId(7)])];
        app.world_mut().resource_mut::<HudFeed>().view = Arc::new(hidden);
        app.update();
        assert!(roots(&mut app).is_empty());

        app.world_mut().resource_mut::<HudFeed>().selected = vec![EntityId(7)];
        app.update();
        assert_eq!(roots(&mut app).len(), 1);
        app.world_mut().resource_mut::<HudConfig>().gui = Some(GuiLayout::default());
        app.insert_resource(UiProfile::FleetOps);
        app.update();
        assert!(roots(&mut app).is_empty());
        assert!(
            app.world()
                .resource::<StatusUiState>()
                .fallback_card()
                .is_none()
        );
    }
}
