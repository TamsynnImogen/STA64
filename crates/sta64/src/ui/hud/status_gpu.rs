//! Status widgets over the existing stock or LCARS information panel.
//! This owns its descendants and never alters the stock info/resource trees.

use bevy::picking::{Pickable, events::PointerClick, pointer::PointerButton};
use bevy::prelude::*;
use sta_sim::command::{BuildQueueId, CommandKind, Order};
use sta_sim::{EntityId, Fixed};

use super::layout::R;
use super::model::{self, HudFeed, Info};
use super::status::{self, JobState, QueueItem, SelectedStatus, StatusFeed};
use super::stock::{self, layout::StockLayout};
use super::{HudConfig, HudLayoutRes, HudRoot};
use crate::sim_link::CommandQueue;
use crate::ui::gui_sprites::{GuiImage, GuiImageSampling};
use crate::ui::notices::Tooltip;
use crate::ui::profile::UiProfile;
use crate::ui::sprite_font::{SpriteText, SpriteTextStyle, sprite_text_styled};
use crate::ui::theme::Theme;

#[derive(Component)]
pub(super) struct StatusRoot;

#[derive(Component, Clone, Copy)]
pub(super) struct QueueCancel {
    builder: EntityId,
    slot: u32,
    job: BuildQueueId,
}

#[derive(Message)]
pub(super) struct QueueClick(Entity);

pub(super) fn on_queue_click(
    mut click: On<PointerClick>,
    cells: Query<(), With<QueueCancel>>,
    lifecycle: Option<Res<crate::platform::LifecycleGate>>,
    mut clicks: MessageWriter<QueueClick>,
) {
    if cells.contains(click.entity) {
        click.propagate(false);
        // Armada II's build wireframe uses the left_click binding (static executable and Input.map).
        if click.button == PointerButton::Primary
            && lifecycle.is_none_or(|gate| gate.is_foreground())
        {
            clicks.write(QueueClick(click.entity));
        }
    }
}

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
    speed_queue: bool,
    progress: Option<R>,
    cargo_bar: Option<R>,
    lcars_card: Option<R>,
}

#[derive(Clone, Debug, PartialEq)]
struct QueueShape {
    queue_id: Option<BuildQueueId>,
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
    let style = if stock { style.stock_fallback() } else { style };
    let wrapper = commands
        .spawn((
            Node {
                overflow: Overflow::clip(),
                ..rect(r)
            },
            Pickable::IGNORE,
            ChildOf(parent),
        ))
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

/// Number of columns and square size for the compact wireframe card.
fn queue_grid(width: f32, count: usize) -> (usize, f32, usize) {
    let inner = (width - 16.0).max(0.0);
    let columns = (((inner + 4.0) / 68.0).floor() as usize).clamp(1, 5);
    let cell = ((inner - (columns - 1) as f32 * 4.0) / columns as f32).clamp(0.0, 64.0);
    let rows = count.div_ceil(columns);
    (columns, cell, rows)
}

fn queue_card_height(width: f32, count: usize, extra_rows: usize) -> f32 {
    let (_, cell, rows) = queue_grid(width, count);
    rows as f32 * (cell + 12.0) + extra_rows as f32 * 26.0 + 10.0
}

fn stock_shape(
    selected: EntityId,
    status: &SelectedStatus,
    layout: &StockLayout,
    info: &Info,
    armada_one: bool,
) -> Shape {
    let variant = stock::layout::info_variant(info);
    let origin = layout.info_origin(variant);
    let mut slots: Vec<_> = (0..10)
        .filter_map(|i| {
            layout
                .at(origin, &format!("infoBuildQueue_{i}"))
                .map(|r| (i, r))
        })
        .collect();
    let mut speed_queue = false;
    let mut progress = layout.at(origin, "infoProgressBar");
    if armada_one && slots.is_empty() && !status.queue.is_empty() {
        slots = stock::a1::speed::queue_slots(layout);
        if let Some((_, head)) = slots.first() {
            speed_queue = true;
            let height = (head.h * 0.12).max(3.0);
            progress = Some(R::new(head.x, head.y + head.h - height, head.w, height));
        }
    }
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
                let jobs = if fallback_queue {
                    status.queue.len().min(10)
                } else if fallback_progress {
                    1
                } else {
                    0
                };
                let h = queue_card_height(
                    panel.w,
                    jobs,
                    usize::from(fallback_cargo) + usize::from(fallback_construction),
                );
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
                queue_id: q.queue_id,
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
        speed_queue,
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
    let jobs = status.queue.len().min(10);
    let extras = usize::from(status.cargo.is_some()) + usize::from(status.construction.is_some());
    let height = queue_card_height(panel.w, jobs, extras);
    let card = (jobs + extras > 0).then(|| {
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
                queue_id: q.queue_id,
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
        speed_queue: false,
        progress: None,
        cargo_bar: None,
        lcars_card: card,
    }
}

fn queue_art(
    commands: &mut Commands,
    root: Entity,
    r: R,
    feed: &HudFeed,
    item: &QueueItem,
    build_icon: bool,
) {
    let officer_upgrade = feed
        .content
        .as_deref()
        .and_then(|db| db.unit(item.def))
        .is_some_and(|def| def.a1_officer_upgrade_gain.is_some());
    if (build_icon || officer_upgrade)
        && let Some(odf) = item.odf.as_deref()
    {
        let mut art = GuiImage::new("");
        art.names = model::build_sprite_names(item.model.as_deref().unwrap_or_default(), odf);
        commands.spawn((rect(r), art, Pickable::IGNORE, ChildOf(root)));
    } else if let Some(model) = item.model.as_deref() {
        stock::queue_wireframe(commands, root, r, model);
    }
}

fn queue_button(
    commands: &mut Commands,
    root: Entity,
    r: R,
    builder: EntityId,
    slot: usize,
    item: &QueueItem,
) {
    let (Some(job), Ok(slot)) = (item.queue_id, u32::try_from(slot)) else {
        return;
    };
    commands.spawn((
        rect(r),
        Pickable::default(),
        QueueCancel { builder, slot, job },
        Tooltip::new(crate::ui::i18n::trf("Cancel {0} from queue", &[&item.name])),
        ChildOf(root),
    ));
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
    for (index, (item, (slot_index, r))) in status.queue.iter().zip(&shape.slots).enumerate() {
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
        queue_art(commands, root, *r, feed, item, shape.speed_queue);
        queue_button(commands, root, *r, shape.selected, index, item);
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
            layout
                .name("infoSingleConstructionBar")
                .or(Some("large_construction_bar")),
            layout
                .colour("infoSingleConstructionBarColor")
                .unwrap_or(theme.warn),
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
        // Thin native bars keep their fill; add the optional numeric label
        // only when a full line fits inside the authored rectangle.
        if r.h >= theme.micro_size.max(16.0) {
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
    let (columns, cell, rows) = queue_grid(card.w, status.queue.len().min(10));
    for (i, item) in status.queue.iter().take(10).enumerate() {
        let x = card.x + 8.0 + (i % columns) as f32 * (cell + 4.0);
        let top = y + (i / columns) as f32 * (cell + 12.0);
        queue_art(
            commands,
            root,
            R::new(x, top, cell, cell),
            feed,
            item,
            false,
        );
        queue_button(
            commands,
            root,
            R::new(x, top, cell, cell),
            shape.selected,
            i,
            item,
        );
        if let Some(value) = item.progress {
            let bar = R::new(x, top + cell + 2.0, cell, 3.0);
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
    }
    y += rows as f32 * (cell + 12.0);
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
        let armada_one = profile.as_deref() == Some(&UiProfile::Armada1);
        stock_shape(
            info.id,
            &status,
            stock_layout,
            &Info::Single(info.clone()),
            armada_one,
        )
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

/// A queue cell is bound to the job shown when it was drawn. Recheck the
/// current selection and job id so a stale cell cannot cancel a shifted slot.
#[allow(clippy::too_many_arguments)]
pub(super) fn press_queue(
    mut clicks: MessageReader<QueueClick>,
    cells: Query<&QueueCancel>,
    feed: Res<HudFeed>,
    source: Res<StatusFeed>,
    avatar: Option<Res<super::avatar::ChoiceState>>,
    cheat: Option<Res<crate::cheats::input::CheatInput>>,
    menu: Option<Res<crate::ui::ingame_menu::InGameMenu>>,
    scene: Option<Res<crate::frontend::scene::MenuScene>>,
    cine: Option<Res<crate::ui::presentation::CineractiveState>>,
    local: Option<Res<crate::render::units::LocalPlayer>>,
    mut commands: ResMut<CommandQueue>,
) {
    if scene.is_some()
        || menu.is_some_and(|m| m.open)
        || cine.is_some_and(|c| c.on)
        || avatar.is_some_and(|a| a.is_open())
        || cheat.is_some_and(|c| c.open || c.blocked_this_frame)
        || local.is_some_and(|p| p.0 != Some(feed.player))
    {
        clicks.read().for_each(drop);
        return;
    }
    let mut clicked = clicks.read().filter_map(|QueueClick(entity)| cells.get(*entity).ok()).peekable();
    if clicked.peek().is_none() {
        return;
    }
    let [builder] = feed.selected.as_slice() else {
        return;
    };
    let selected = status::selected(&feed, &source);
    let mut sent = Vec::new();
    for cell in clicked {
        if cell.builder != *builder || sent.contains(&cell.job) {
            continue;
        }
        let Some(item) = selected.queue.get(cell.slot as usize) else {
            continue;
        };
        if item.queue_id != Some(cell.job) {
            continue;
        }
        sent.push(cell.job);
        commands.0.push(CommandKind::Order(Order::CancelBuild {
            builder: cell.builder,
            slot: cell.slot,
            job: cell.job,
        }));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sta_content::gui_cfg::{GuiLayout, Panel, Rect};
    use sta_sim::ContentKey;
    use sta_sim::production::view::{ProductionJobView, ProductionView};
    use sta_sim::view::UnitFlags;
    use sta_sim::{PlayerId, UnitView, Vec3, WorldView};

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
                    queue_id: Some(BuildQueueId(1)),
                    def: ContentKey(1),
                    odf: Some("ship.odf".into()),
                    model: Some("models/ship.sod".into()),
                    name: "Ship".into(),
                    state: JobState::Building,
                    progress: Some(0.5),
                },
                QueueItem {
                    queue_id: Some(BuildQueueId(2)),
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

    fn click_app(stock: bool) -> (App, Entity) {
        use bevy::ecs::system::RunSystemOnce;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<QueueClick>()
            .add_observer(on_queue_click)
            .add_systems(Update, press_queue)
            .insert_resource(CommandQueue::default());
        let mut view = WorldView::default();
        view.units.push(UnitView {
            id: EntityId(7),
            def: ContentKey::NONE,
            owner: PlayerId(1),
            pos: Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2],
            shields: [[Fixed::ZERO; 2]; 4],
            flags: UnitFlags::SELECTABLE,
            progress: None,
            crew: None,
            energy: None,
            autonomy: None,
        });
        view.visibility.push((PlayerId(1), vec![EntityId(7)]));
        app.insert_resource(HudFeed {
            view: Arc::new(view),
            player: PlayerId(1),
            selected: vec![EntityId(7)],
            ..Default::default()
        });
        app.insert_resource(StatusFeed {
            production: vec![ProductionView {
                builder: EntityId(7),
                portal: None,
                research: None,
                officer_upgrades: None,
                jobs: vec![
                    ProductionJobView {
                        queue_id: Some(BuildQueueId(1)),
                        def: ContentKey(1),
                        progress: Fixed::ZERO,
                        paid: true,
                        started: true,
                    },
                    ProductionJobView {
                        queue_id: Some(BuildQueueId(2)),
                        def: ContentKey(2),
                        progress: Fixed::ZERO,
                        paid: false,
                        started: false,
                    },
                ],
                site: None,
            }],
            ..Default::default()
        });
        let root = app.world_mut().spawn_empty().id();
        let status = queued();
        let selected = info(true);
        let theme = Theme::default();
        if stock {
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
                    w: 96,
                    h: 96,
                },
            );
            let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
            let shape = stock_shape(EntityId(7), &status, &layout, &selected, false);
            app.world_mut()
                .run_system_once(move |mut commands: Commands, feed: Res<HudFeed>| {
                    draw_stock(
                        &mut commands,
                        root,
                        &feed,
                        &status,
                        &shape,
                        &layout,
                        &theme,
                        &selected,
                    );
                })
                .unwrap();
        } else {
            let layout = HudLayoutRes(super::super::layout::HudLayout::new(None, 16.0 / 9.0));
            let shape = lcars_shape(EntityId(7), &status, &layout, &selected, &theme);
            app.world_mut()
                .run_system_once(move |mut commands: Commands, feed: Res<HudFeed>| {
                    draw_lcars(&mut commands, root, &feed, &status, &shape, &theme);
                })
                .unwrap();
        }
        let mut q = app.world_mut().query::<(Entity, &QueueCancel)>();
        let buttons: Vec<_> = q
            .iter(app.world())
            .map(|(entity, cell)| (entity, cell.slot))
            .collect();
        assert_eq!(buttons.len(), 2);
        let clicked = buttons.into_iter().find(|(_, slot)| *slot == 1).unwrap().0;
        (app, clicked)
    }

    fn click(app: &mut App, entity: Entity, button: PointerButton) {
        use bevy::picking::{backend::HitData, events::Pointer, pointer::{Location, PointerId}};
        let camera = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(PointerClick {
            entity,
            pointer: Pointer::new(PointerId::Mouse, Location {
                target: bevy::camera::NormalizedRenderTarget::None { width: 1, height: 1 },
                position: Vec2::ZERO,
            }),
            button,
            hit: HitData::new(camera, 0.0, None, None),
            duration: std::time::Duration::ZERO,
            count: 1,
        });
    }

    #[test]
    fn stock_and_lcars_cells_cancel_one_matching_job() {
        for stock in [true, false] {
            let (mut app, clicked) = click_app(stock);
            click(&mut app, clicked, PointerButton::Secondary);
            app.update();
            assert!(app.world().resource::<CommandQueue>().0.is_empty());
            click(&mut app, clicked, PointerButton::Primary);
            click(&mut app, clicked, PointerButton::Primary);
            app.update();
            let queue = &app.world().resource::<CommandQueue>().0;
            assert_eq!(queue.len(), 1);
            assert!(matches!(
                queue[0],
                CommandKind::Order(Order::CancelBuild {
                    builder: EntityId(7),
                    slot: 1,
                    job: BuildQueueId(2)
                })
            ));
        }
    }

    #[test]
    fn stock_and_lcars_reject_foreign_or_stale_queue_cells() {
        for stock in [true, false] {
            let (mut app, clicked) = click_app(stock);
            Arc::make_mut(&mut app.world_mut().resource_mut::<HudFeed>().view).units[0].owner =
                PlayerId(2);
            click(&mut app, clicked, PointerButton::Primary);
            app.update();
            assert!(app.world().resource::<CommandQueue>().0.is_empty());

            Arc::make_mut(&mut app.world_mut().resource_mut::<HudFeed>().view).units[0].owner =
                PlayerId(1);
            app.world_mut().resource_mut::<StatusFeed>().production[0]
                .jobs
                .swap(0, 1);
            click(&mut app, clicked, PointerButton::Primary);
            app.update();
            assert!(app.world().resource::<CommandQueue>().0.is_empty());
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
        let shape = stock_shape(EntityId(7), &queued(), &layout, &info(true), false);
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
            stock_shape(EntityId(7), &queued(), &layout, &info(true), false)
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
        let shape = stock_shape(EntityId(7), &status, &layout, &info(true), false);
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
            stock_shape(EntityId(7), &queued(), &layout, &info(true), false)
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
            stock_shape(EntityId(7), &queued(), &layout, &info(true), false)
                .lcars_card
                .is_none()
        );
    }

    #[test]
    fn thin_native_cargo_bar_keeps_its_fill_without_overflowing_text() {
        use bevy::ecs::system::RunSystemOnce;
        for (height, expect_label) in [(12, false), (64, true)] {
            let mut cfg = GuiLayout {
                canvas: (1600, 1200),
                ..Default::default()
            };
            cfg.panels.insert(
                "info".into(),
                Panel {
                    area: Some(Rect {
                        x: 20,
                        y: 800,
                        w: 800,
                        h: 300,
                    }),
                    ..Default::default()
                },
            );
            cfg.rects.insert(
                "infoSingleDilithiumBarArea".into(),
                Rect {
                    x: 100,
                    y: 100,
                    w: 400,
                    h: height,
                },
            );
            let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
            let status = SelectedStatus {
                cargo: Some(status::CargoStatus {
                    amount: Fixed::from_int(50),
                    capacity: Fixed::from_int(100),
                    resource: Some(0),
                    fraction: 0.5,
                }),
                ..Default::default()
            };
            let selected = info(false);
            let shape = stock_shape(EntityId(7), &status, &layout, &selected, false);
            let mut world = World::new();
            let root = world.spawn_empty().id();
            world
                .run_system_once(move |mut commands: Commands| {
                    draw_stock(
                        &mut commands,
                        root,
                        &HudFeed::default(),
                        &status,
                        &shape,
                        &layout,
                        &Theme::default(),
                        &selected,
                    );
                })
                .unwrap();
            let mut values = world.query::<(Entity, &LiveValue)>();
            let values: Vec<_> = values
                .iter(&world)
                .map(|(entity, value)| (entity, *value))
                .collect();
            assert_eq!(
                values
                    .iter()
                    .filter(|(_, value)| matches!(value, LiveValue::Cargo(_)))
                    .count(),
                1
            );
            let labels: Vec<_> = values
                .iter()
                .filter(|(_, value)| matches!(value, LiveValue::CargoAmount))
                .collect();
            assert_eq!(labels.len(), usize::from(expect_label));
            if let Some((entity, _)) = labels.first() {
                let parent = world.get::<ChildOf>(*entity).unwrap().parent();
                assert_eq!(
                    world.get::<Node>(parent).unwrap().overflow,
                    Overflow::clip()
                );
            }
        }
    }

    #[test]
    fn stock_queue_draws_authored_background_five_wireframes_and_native_progress_art() {
        use bevy::ecs::system::RunSystemOnce;

        let mut cfg = GuiLayout {
            canvas: (1600, 1200),
            ..Default::default()
        };
        cfg.rects.insert(
            "infoBuildQueue_0".into(),
            Rect {
                x: 10,
                y: 20,
                w: 96,
                h: 96,
            },
        );
        cfg.rects.insert(
            "infoProgressBar".into(),
            Rect {
                x: 10,
                y: 120,
                w: 200,
                h: 12,
            },
        );
        cfg.names
            .insert("infoBuildQueueSlot_0".into(), "slot_background".into());
        cfg.names.insert(
            "infoSingleConstructionBar".into(),
            "custom_build_bar".into(),
        );
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let mut status = queued();
        status.queue.truncate(1);
        let selected = info(true);
        let shape = stock_shape(EntityId(7), &status, &layout, &selected, false);
        let slot_rect = shape.slots[0].1;
        let progress_rect = shape.progress.unwrap();
        let mut world = World::new();
        let root = world.spawn_empty().id();
        world
            .run_system_once(move |mut commands: Commands| {
                draw_stock(
                    &mut commands,
                    root,
                    &HudFeed::default(),
                    &status,
                    &shape,
                    &layout,
                    &Theme::default(),
                    &selected,
                );
            })
            .unwrap();

        let mut images = world.query::<(&GuiImage, &Node)>();
        let images: Vec<_> = images.iter(&world).collect();
        assert_eq!(images.len(), 7); // slot, five layers, progress
        assert!(
            images
                .iter()
                .any(|(image, _)| image.names == ["slot_background"])
        );
        for layer in 1..=5 {
            let name = format!("shipw{layer}");
            let (_, node) = images
                .iter()
                .find(|(image, _)| image.names[0] == name)
                .unwrap();
            assert_eq!(node.left, Val::Px(slot_rect.x));
            assert_eq!(node.top, Val::Px(slot_rect.y));
            assert_eq!(node.width, Val::Px(slot_rect.w));
            assert_eq!(node.height, Val::Px(slot_rect.h));
        }
        assert!(
            images
                .iter()
                .any(|(image, _)| image.names == ["custom_build_bar"])
        );
        assert!(
            images
                .iter()
                .all(|(image, _)| image.names.iter().all(|name| !name.starts_with("b_")))
        );
        let mut clips = world.query::<(&LiveValue, &Node)>();
        let clips: Vec<_> = clips.iter(&world).collect();
        assert!(clips.iter().any(|(value, node)| matches!(value, LiveValue::QueueProgress(0, width) if *width == progress_rect.w) && node.width == Val::Px(progress_rect.w * 0.5)));
        let mut labels = world.query::<&SpriteText>();
        assert_eq!(labels.iter(&world).count(), 0);
    }

    #[test]
    fn fallback_queue_uses_bounded_grid_with_live_cell_progress() {
        use bevy::ecs::system::RunSystemOnce;

        let mut status = queued();
        let second = status.queue[1].clone();
        status.queue.extend((0..8).map(|_| second.clone()));
        let card = R::new(100.0, 200.0, 400.0, queue_card_height(400.0, 10, 0));
        let (cols, cell, rows) = queue_grid(card.w, status.queue.len());
        assert_eq!((cols, cell, rows), (5, 64.0, 2));
        assert_eq!(card.h, 162.0);
        let shape = Shape {
            selected: EntityId(7),
            stock: false,
            queue: Vec::new(),
            cargo: false,
            cargo_resource: None,
            construction: false,
            slots: Vec::new(),
            speed_queue: false,
            progress: None,
            cargo_bar: None,
            lcars_card: Some(card),
        };
        let mut world = World::new();
        let root = world.spawn_empty().id();
        world
            .run_system_once(move |mut commands: Commands| {
                draw_lcars(
                    &mut commands,
                    root,
                    &HudFeed::default(),
                    &status,
                    &shape,
                    &Theme::default(),
                );
            })
            .unwrap();
        let mut images = world.query::<(&GuiImage, &Node)>();
        let images: Vec<_> = images.iter(&world).collect();
        assert_eq!(images.len(), 5); // only the modeled first job has five layers
        let (_, first) = images
            .iter()
            .find(|(image, _)| image.names[0] == "shipw1")
            .unwrap();
        assert_eq!(first.left, Val::Px(108.0));
        assert_eq!(first.top, Val::Px(205.0));
        assert_eq!(first.width, Val::Px(64.0));
        let mut clips = world.query::<(&LiveValue, &Node)>();
        assert!(clips.iter(&world).any(
            |(value, node)| matches!(value, LiveValue::QueueProgress(0, width) if *width == 64.0)
                && node.width == Val::Px(32.0)
        ));
        let mut labels = world.query::<&SpriteText>();
        assert_eq!(labels.iter(&world).count(), 0);
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
            stock_shape(EntityId(7), &before, &layout, &info(true), false),
            stock_shape(EntityId(7), &after, &layout, &info(true), false)
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
            production: Vec::new(),
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
