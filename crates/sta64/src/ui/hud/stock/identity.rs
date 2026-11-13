//! Optional selected-panel companion rows for the chosen craft identity.

use bevy::prelude::*;

use super::super::super::layout::R;
use super::super::super::model::Single;
use super::super::layout::StockLayout;
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
    medium_area: &str,
    tall_area: &str,
    legacy_area: &str,
    medium_colour: &str,
    tall_colour: &str,
    global_colour: &str,
) {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return;
    };
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
        value.to_owned(),
        colour,
        theme.small_size,
        None,
    );
    commands.entity(entity).insert(IdentityText(field));
}

pub(super) fn spawn(
    commands: &mut Commands,
    root: Entity,
    layout: &StockLayout,
    theme: &Theme,
    origin: Vec2,
    single: &Single,
) {
    row(
        commands,
        root,
        layout,
        theme,
        origin,
        single.builder,
        single.captain.as_deref(),
        IdentityField::Captain,
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
        single.builder,
        single.registry.as_deref(),
        IdentityField::Registry,
        "infoSingleRegistryTextArea",
        "infoBuildRegistryTextArea",
        "shipRegistry",
        "infoSingleRegistryTextColor",
        "infoBuildRegistryTextColor",
        "shipRegistryColor",
    );
}
