//! The Armada I single-selection info panel (`infoSingle*` keys, read from data).
//!
//! Drawn for one selected unit: the race icon (a bar and the race's emblem, read from the
//! player's sprite table at run time, never shipped), the class and name labels, the wireframe,
//! the shield bar (the hull fraction when the unit has no shields: not measured), the
//! construction bar while the unit is being built or else the energy bar, and the crew readout
//! with its dot, coloured by the race's `crewYellowStatus` and `crewRedStatus`.
//! The sprite bars are drawn clipped to their fill, so the art is the game's own.
//!
//! Not in the unit view yet, hence not drawn (TODO lines in the PR): the officers readout, the
//! dilithium bar. Multi-selection (`infoMulti*`) and the system icons (`infoSystem*`) are later
//! packages; the Armada II group tiles are not used.

use bevy::{picking::Pickable, prelude::*};
use sta_sim::Fixed;

use super::super::{
    infobar::InfoStyle,
    layout::StockLayout,
};
use super::super::info::{StockBar, node, picture, text, wireframe_layers};
use crate::ui::{
    gui_sprites::{GuiImage, GuiImageSampling},
    hud::{
        layout::R,
        model::{self, HudFeed, Info, Single},
    },
    theme::Theme,
};

/// What the panel shows besides [`Single`]: values the shared model does not carry.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Extra {
    /// Energy as 0..1.
    pub energy: Option<f32>,
    /// Whole crew aboard and its fraction of the maximum.
    pub crew: Option<(i64, f32)>,
    /// The unit's race name from its definition (empty when unknown).
    pub race: String,
}

/// The crew readout's text, patched in place as the crew changes.
#[derive(Component)]
pub(in crate::ui::hud) struct CrewText;

/// What decides whether the card must be rebuilt: which fields exist, the race and the crew dot's
/// colour. The values themselves (energy, crew count) are patched in place by `info::update`, so
/// the buttons of the grid are not respawned when they change.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shape {
    pub energy: bool,
    pub crew: bool,
    pub race: String,
    pub dot: Option<Color>,
}

impl Extra {
    pub(in crate::ui::hud) fn shape(&self, style: &InfoStyle) -> Shape {
        Shape {
            energy: self.energy.is_some(),
            crew: self.crew.is_some(),
            race: self.race.clone(),
            dot: self.crew.map(|(_, ratio)| style.color(&self.race, Some(ratio))),
        }
    }
}

/// The extra values of a single selection, `None` for any other selection.
pub(in crate::ui::hud) fn extra(feed: &HudFeed) -> Option<Extra> {
    let sel = feed.selected_units();
    let [u] = sel.as_slice() else { return None };
    let frac = |v: [Fixed; 2]| if v[1] > Fixed::ZERO { (v[0].to_f32() / v[1].to_f32()).clamp(0.0, 1.0) } else { 0.0 };
    Some(Extra {
        energy: u.energy.map(frac),
        crew: u.crew.map(|c| (c[0].to_f32() as i64, frac(c))),
        race: feed.content.as_deref().and_then(|db| db.unit(u.def)).map(|d| d.race.clone()).unwrap_or_default(),
    })
}

/// Emblem sprite candidates for a race name or stem (`federation`, `fed`, ...).
pub(in crate::ui::hud) fn emblem_names(race: &str) -> Vec<String> {
    let race = race.trim().to_ascii_lowercase();
    let mut names = Vec::new();
    if !race.is_empty() {
        names.push(format!("{race}_icon"));
        for full in ["federation", "klingon", "romulan", "borg"] {
            let name = format!("{full}_icon");
            if race.get(..3).is_some_and(|p| full.starts_with(p)) && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

fn key_rect(layout: &StockLayout, origin: Vec2, keys: &[&str]) -> Option<R> {
    keys.iter().find_map(|k| layout.at(origin, k))
}

/// A sprite bar: the sprite clipped to `value` of its width over a dark track. `kind` makes the
/// clipped part follow live values.
fn sprite_bar(commands: &mut Commands, root: Entity, r: R, sprite: &str, value: f32, kind: Option<StockBar>, tint: Color) {
    let track = commands
        .spawn((node(r), BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)), Pickable::IGNORE, ChildOf(root)))
        .id();
    let mut clip = commands.spawn((
        Node { width: Val::Px(r.w * value.clamp(0.0, 1.0)), height: Val::Percent(100.0), overflow: Overflow::clip(), ..default() },
        Pickable::IGNORE,
        ChildOf(track),
        children![(
            Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Px(r.w), height: Val::Px(r.h), ..default() },
            Pickable::IGNORE,
            GuiImage::new(sprite).with_color(tint).with_sampling(GuiImageSampling::Linear),
        )],
    ));
    if let Some(kind) = kind {
        clip.insert(kind);
    }
}

/// The panel's dynamic content: the resource strip, and for one selected unit its card.
pub(in crate::ui::hud) fn spawn(commands: &mut Commands, root: Entity, layout: &StockLayout, feed: &HudFeed, theme: &Theme, style: &InfoStyle) {
    super::super::info::spawn_resource(commands, root, layout, feed, theme);
    let Some(area) = layout.panel("info") else { return };
    let origin = Vec2::new(area.x, area.y);
    let Info::Single(single) = model::info(feed) else { return };
    let extra = extra(feed).unwrap_or_default();
    single_card(commands, root, layout, feed, theme, style, origin, &single, &extra);
}

#[allow(clippy::too_many_arguments)]
fn single_card(
    commands: &mut Commands,
    root: Entity,
    layout: &StockLayout,
    feed: &HudFeed,
    theme: &Theme,
    style: &InfoStyle,
    origin: Vec2,
    single: &Single,
    extra: &Extra,
) {
    let pic = |commands: &mut Commands, key: &str, area: &str| {
        if let (Some(name), Some(r)) = (layout.name(key), layout.at(origin, area)) {
            picture(commands, root, r, name, 0);
        }
    };
    // Race icon: the bar, then the emblem in its display area.
    pic(commands, "infoSingleRaceIcon", "infoSingleRaceIconArea");
    if let Some(r) = layout.at(origin, "infoSingleRaceIconDisplayArea") {
        let mut image = GuiImage::new("").with_sampling(GuiImageSampling::Linear);
        image.names = emblem_names(&extra.race);
        commands.entity(root).with_child((node(r), Pickable::IGNORE, image));
    }
    pic(commands, "infoSingleNameTextBackground", "infoSingleNameTextBackgroundArea");
    let colour = |key: &str, fallback: Color| layout.colour(key).unwrap_or(fallback);
    if let Some(r) = key_rect(layout, origin, &["infoSingleNameTextInformationArea", "infoSingleNameTextArea"]) {
        text(commands, root, r, single.name.clone(), colour("infoSingleNameTextColor", theme.text), theme.micro_size, None);
    }
    if let Some(r) = key_rect(layout, origin, &["infoSingleClassTextInformationArea", "infoSingleClassTextArea"]) {
        text(commands, root, r, single.class.clone(), colour("infoSingleClassTextColor", theme.text_dim), theme.micro_size, None);
    }
    if let Some(r) = layout.at(origin, "infoSingleWireframeIconArea")
        && let Some(unit) = feed.view.unit(single.id)
        && let Some(def) = feed.content.as_deref().and_then(|db| db.unit(unit.def))
    {
        wireframe_layers(commands, root, r, &def.model);
    }
    // Bars. The shield bar shows the hull fraction when the unit has no shields.
    let tint = |key: &str| layout.colour(key).unwrap_or(Color::WHITE);
    if let Some(r) = layout.at(origin, "infoSingleShieldBarArea") {
        let name = layout.name("infoSingleShieldBar").unwrap_or_default();
        let (value, kind) = match single.shield {
            Some(s) => (s, StockBar::Shield(r.w)),
            None => (single.hull, StockBar::Hull(r.w)),
        };
        sprite_bar(commands, root, r, name, value, Some(kind), Color::WHITE);
    }
    if let Some(progress) = single.progress {
        if let Some(r) = layout.at(origin, "infoSingleConstructionBarArea") {
            let name = layout.name("infoSingleConstructionBar").unwrap_or_default();
            sprite_bar(commands, root, r, name, progress, Some(StockBar::Progress(r.w)), tint("infoSingleConstructionBarColor"));
        }
    } else if let (Some(energy), Some(r)) = (extra.energy, layout.at(origin, "infoSingleEnergyBarArea")) {
        let name = layout.name("infoSingleEnergyBar").unwrap_or_default();
        sprite_bar(commands, root, r, name, energy, Some(StockBar::Energy(r.w)), tint("infoSingleEnergyBarColor"));
    }
    // Crew: label bar, label, value and the dot.
    if let Some((now, ratio)) = extra.crew {
        pic(commands, "infoSingleCrewTextBackground", "infoSingleCrewTextBackgroundArea");
        if let Some(r) = layout.at(origin, "infoSingleCrewTextInformationArea") {
            let entity = text(commands, root, r, now.to_string(), theme.text, theme.micro_size, None);
            commands.entity(entity).insert(CrewText);
        }
        if let (Some(name), Some(r)) = (layout.name("infoSingleCrewDot"), layout.at(origin, "infoSingleCrewDotArea")) {
            let colour = style.color(&extra.race, Some(ratio));
            commands.entity(root).with_child((node(r), Pickable::IGNORE, GuiImage::new(name).with_color(colour)));
        }
    }
}
