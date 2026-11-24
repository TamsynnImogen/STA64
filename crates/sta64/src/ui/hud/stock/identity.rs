//! Optional selected-panel companion rows for the chosen craft identity.

use bevy::prelude::*;

use super::super::super::layout::R;
use super::super::super::model::Single;
use super::super::layout::StockLayout;
use super::StockText;
use crate::ui::theme::Theme;

/// Diagnostic identity of an actual `SpriteText` row, useful to inspect the HUD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::hud) enum IdentityField {
    Captain,
    Registry,
}

#[derive(Component, Debug, Clone, Copy)]
pub(in crate::ui::hud) struct IdentityText(#[allow(dead_code)] pub IdentityField);

fn area(
    layout: &StockLayout,
    origin: Vec2,
    builder: bool,
    medium: &str,
    tall: &str,
    legacy: &str,
) -> Option<R> {
    let key = if builder && layout.raw_rect(tall).is_some() {
        tall
    } else if layout.raw_rect(medium).is_some() {
        medium
    } else {
        legacy
    };
    layout.at(origin, key)
}

fn colour(
    layout: &StockLayout,
    builder: bool,
    medium: &str,
    tall: &str,
    global: &str,
    fallback: Color,
) -> Color {
    (if builder { layout.colour(tall) } else { None })
        .or_else(|| layout.colour(medium))
        .or_else(|| layout.colour(global))
        .or_else(|| layout.colour("infoTextColor"))
        .unwrap_or(fallback)
}

#[allow(clippy::too_many_arguments)]
fn row(
    commands: &mut Commands,
    root: Entity,
    layout: &StockLayout,
    theme: &Theme,
    origin: Vec2,
    builder: bool,
    value: Option<&str>,
    field: IdentityField,
    live: Option<StockText>,
    medium_area: &str,
    tall_area: &str,
    legacy_area: &str,
    medium_colour: &str,
    tall_colour: &str,
    global_colour: &str,
) {
    let value = value.filter(|value| !value.trim().is_empty());
    if value.is_none() && live.is_none() {
        return;
    }
    let Some(rect) = area(layout, origin, builder, medium_area, tall_area, legacy_area) else {
        return;
    };
    let colour = colour(
        layout,
        builder,
        medium_colour,
        tall_colour,
        global_colour,
        theme.text,
    );
    let entity = super::text(
        commands,
        root,
        rect,
        value.unwrap_or_default().to_owned(),
        colour,
        theme.small_size,
        None,
    );
    let mut entity = commands.entity(entity);
    entity.insert(IdentityText(field));
    if let Some(live) = live {
        entity.insert(live);
    }
}

pub(super) fn spawn(
    commands: &mut Commands,
    root: Entity,
    layout: &StockLayout,
    theme: &Theme,
    origin: Vec2,
    single: &Single,
    construction_rig: bool,
) {
    row(
        commands,
        root,
        layout,
        theme,
        origin,
        layout.info_uses_builder(single),
        single.captain.as_deref(),
        IdentityField::Captain,
        construction_rig.then_some(StockText::RigJob(single.id)),
        "infoSingleCaptainTextArea",
        "infoBuildCaptainTextArea",
        "captainName",
        "infoSingleCaptainTextColor",
        "infoBuildCaptainTextColor",
        "captainNameColor",
    );
    row(
        commands,
        root,
        layout,
        theme,
        origin,
        layout.info_uses_builder(single),
        single.registry.as_deref(),
        IdentityField::Registry,
        None,
        "infoSingleRegistryTextArea",
        "infoBuildRegistryTextArea",
        "shipRegistry",
        "infoSingleRegistryTextColor",
        "infoBuildRegistryTextColor",
        "shipRegistryColor",
    );
}
