//! The Armada I single-selection info panel (`infoSingle*` keys, read from data).
//!
//! Drawn for one selected unit: the race icon (a bar and the race's emblem, read from the
//! player's sprite table at run time, never shipped), the class and name labels, the wireframe,
//! the shield bar (the hull fraction when the unit has no shields: not measured), the
//! construction bar while the unit is being built or else the energy bar, and the crew readout
//! with its dot, coloured by the race's `crewYellowStatus` and `crewRedStatus`.
//! The sprite bars are drawn clipped to their fill, so the art is the game's own.
//!
//! The resource bar follows the shared node feed. Multi-selection (`infoMulti*`) and the
//! system icons (`infoSystem*`) are later
//! packages; the Armada II group tiles are not used.

use bevy::{picking::Pickable, prelude::*};
use sta_sim::{EntityId, Fixed};

use super::super::{
    infobar::InfoStyle,
    layout::StockLayout,
};
use crate::ui::hud::readouts::{self, Readout};
use super::super::info::{StockBar, bar_colours, node, picture, place, text, wireframe_layers_of};
use crate::ui::{
    gui_sprites::{GuiImage, GuiImageSampling},
    hud::{
        layout::R,
        model::{self, HudFeed, Info, Single},
    },
    theme::Theme,
};
use crate::viewer::{MapTeamColours, TeamColours, owner_hull_colour};

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

/// The native RaceIcon and ShieldBar read these colours from the selected object on draw.
#[derive(Component, Clone, Copy)]
pub(in crate::ui::hud) enum LiveTint {
    Race(EntityId),
    Shield(EntityId),
}

// Armada I ShieldBar: the filled sprite blends from red at zero to green at full
// strength (read from Armada.exe, static). Use the existing hull display for shieldless units.
fn shield_tint(fraction: f32) -> Color {
    Color::srgb(1.0 - fraction, fraction, 0.0)
}

pub(in crate::ui::hud) fn update_tints(
    feed: Res<HudFeed>,
    palette: Option<Res<TeamColours>>,
    map: Option<Res<MapTeamColours>>,
    mut pictures: Query<(&LiveTint, &mut GuiImage)>,
) {
    for (tint, mut image) in &mut pictures {
        let colour = match *tint {
            LiveTint::Race(id) => feed
                .view
                .unit(id)
                .and_then(|unit| owner_hull_colour(palette.as_deref(), map.as_deref(), unit.owner)),
            LiveTint::Shield(id) => feed.view.unit(id).map(|unit| {
                shield_tint(model::shield_frac(unit).unwrap_or_else(|| model::frac(unit.hull)))
            }),
        }
        .unwrap_or(Color::WHITE);
        if image.color != colour {
            image.color = colour;
        }
    }
}

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

/// Armada I StandardBar draws a tinted track and a clipped fill using the same sprite.
fn sprite_bar(
    commands: &mut Commands,
    root: Entity,
    r: R,
    sprite: &str,
    value: f32,
    (track, fill): (Color, Color),
    kind: StockBar,
) -> Entity {
    let track = commands
        .spawn((
            node(r),
            GuiImage::new(sprite).with_color(track).with_sampling(GuiImageSampling::Linear),
            Pickable::IGNORE,
            ChildOf(root),
        ))
        .id();
    let clip = commands
        .spawn((
            Node { width: Val::Px(r.w * value.clamp(0.0, 1.0)), height: Val::Percent(100.0), overflow: Overflow::clip(), ..default() },
            Pickable::IGNORE,
            kind,
            ChildOf(track),
        ))
        .id();
    commands
        .spawn((
            Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Px(r.w), height: Val::Px(r.h), ..default() },
            Pickable::IGNORE,
            GuiImage::new(sprite).with_color(fill).with_sampling(GuiImageSampling::Linear),
            ChildOf(clip),
        ))
        .id()
}

/// The panel's dynamic content: the resource strip, and for one selected unit its card.
pub(in crate::ui::hud) fn spawn(commands: &mut Commands, root: Entity, layout: &StockLayout, feed: &HudFeed, theme: &Theme, style: &InfoStyle, strings: Option<&crate::ui::i18n::Strings>) {
    super::super::info::spawn_resource(commands, root, layout, feed, theme);
    let Some(area) = layout.panel("info") else { return };
    let origin = Vec2::new(area.x, area.y);
    let Info::Single(single) = model::info(feed) else { return };
    let extra = extra(feed).unwrap_or_default();
    single_card(commands, root, layout, feed, theme, style, origin, &single, &extra, strings);
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
    strings: Option<&crate::ui::i18n::Strings>,
) {
    let pic = |commands: &mut Commands, key: &str, area: &str| {
        if let (Some(name), Some(r)) = (layout.name(key), layout.at(origin, area)) {
            picture(commands, root, r, name, 0);
        }
    };
    // The class caption sits on the first piece of A1's inner identity panel.
    for piece in layout.pieces_in("info", "infoSingleBackground") {
        picture(commands, root, place(origin, piece.rect), &piece.name, piece.frame);
    }
    // RaceIcon draws both layers in the selected owner's colour. A1Compat restores
    // the bar in Roots, where the native insignia and team palette are still available.
    if let Some(r) = layout.at(origin, "infoSingleRaceIconArea") {
        let sprite = layout.name("infoSingleRaceIconBackground")
            .or_else(|| layout.name("infoSingleRaceIcon"))
            .unwrap_or("race_icon_bar");
        commands.entity(root).with_child((node(r), Pickable::IGNORE, GuiImage::new(sprite), LiveTint::Race(single.id)));
    }
    if let Some(r) = layout.at(origin, "infoSingleRaceIconDisplayArea") {
        let mut image = GuiImage::new("").with_sampling(GuiImageSampling::Linear);
        image.names = emblem_names(&extra.race);
        commands.entity(root).with_child((node(r), Pickable::IGNORE, image, LiveTint::Race(single.id)));
    }
    pic(commands, "infoSingleNameTextBackground", "infoSingleNameTextBackgroundArea");
    // Armada.exe's GUI colour lookup supplies black to StandardText for a
    // missing *TextColor. The artwork under these captions is light.
    let colour = |key: &str| layout.colour(key).unwrap_or(Color::BLACK);
    if let Some(r) = key_rect(layout, origin, &["infoSingleNameTextInformationArea", "infoSingleNameTextArea"]) {
        text(commands, root, r, single.name.clone(), colour("infoSingleNameTextColor"), theme.micro_size, None);
    }
    if let Some(r) = key_rect(layout, origin, &["infoSingleClassTextInformationArea", "infoSingleClassTextArea"]) {
        text(commands, root, r, single.class.clone(), colour("infoSingleClassTextColor"), theme.micro_size, None);
    }
    if let Some(r) = layout.at(origin, "infoSingleWireframeIconArea")
        && let Some(unit) = feed.view.unit(single.id)
        && let Some(def) = feed.content.as_deref().and_then(|db| db.unit(unit.def))
    {
        wireframe_layers_of(commands, root, r, &def.model, Some(unit.id), None, Some((&single.name, def.raw.str("verboseTooltip"))));
    }
    // Bars. The shield bar shows the hull fraction when the unit has no shields.
    let def = feed.view.unit(single.id).and_then(|u| feed.content.as_deref().and_then(|db| db.unit(u.def)));
    let node_def = def.filter(|d| crate::ui::hud::status::node_slot(d).is_some());
    let shield_rect = layout.at(origin, "infoSingleShieldBarArea");
    if let Some(def) = node_def {
        super::super::info::spawn_node_bar(commands, root, layout, origin, def, shield_rect, theme, single.id);
    } else if let Some(r) = shield_rect {
        let name = layout.name("infoSingleShieldBar").unwrap_or("large_shield_bar");
        let (value, kind) = match single.shield {
            Some(s) => (s, StockBar::Shield(r.w)),
            None => (single.hull, StockBar::Hull(r.w)),
        };
        let colours = bar_colours(layout, &["infoSingleShieldBarDisableColor"], &["infoSingleShieldBarColor"], shield_tint(value));
        let bar = sprite_bar(commands, root, r, name, value, colours, kind);
        let readout = if single.shield.is_some() { Readout::Shields } else { Readout::Hull };
        readouts::at(commands, root, r, single.id, readout);
        if layout.colour("infoSingleShieldBarColor").is_none() {
            commands.entity(bar).insert(LiveTint::Shield(single.id));
        }
    }
    if let Some(progress) = single.progress {
        if let Some(r) = layout.at(origin, "infoSingleConstructionBarArea") {
            let name = layout.name("infoSingleConstructionBar").unwrap_or_default();
            let colours = bar_colours(layout, &["infoSingleConstructionBarDisableColor"], &["infoSingleConstructionBarColor"], Color::WHITE);
            sprite_bar(commands, root, r, name, progress, colours, StockBar::Progress(r.w));
            readouts::at(commands, root, r, single.id, Readout::Construction);
        }
    } else if let (Some(energy), Some(r)) = (extra.energy, layout.at(origin, "infoSingleEnergyBarArea")) {
        let name = layout.name("infoSingleEnergyBar").unwrap_or("large_energy_bar");
        let colours = bar_colours(layout, &["infoSingleEnergyBarDisableColor"], &["infoSingleEnergyBarColor"], Color::WHITE);
        sprite_bar(commands, root, r, name, energy, colours, StockBar::Energy(r.w));
        readouts::at(commands, root, r, single.id, Readout::Energy);
    }
    // Crew: label bar, label, value and the dot.
    if let Some((now, ratio)) = extra.crew {
        pic(commands, "infoSingleCrewTextBackground", "infoSingleCrewTextBackgroundArea");
        let crew_colour = colour("infoSingleCrewTextColor");
        // UI-48: the label on its bar, a string-table key (`GUI_SD_TEXT_CREW_LABEL` in stock).
        if let (Some(key), Some(r)) = (layout.name("infoSingleCrewTextLabel"), layout.at(origin, "infoSingleCrewTextLabelArea")) {
            let label = strings.map(|s| s.game_text(key)).filter(|t| *t != key).unwrap_or("CREW").to_owned();
            text(commands, root, r, label, crew_colour, theme.micro_size, None);
            readouts::at(commands, root, r, single.id, Readout::Crew);
        }
        if let Some(r) = layout.at(origin, "infoSingleCrewTextInformationArea") {
            // The compatibility renderer keeps the live crew amount green.
            let entity = text(commands, root, r, now.to_string(), Color::srgb(0.0, 1.0, 0.0), theme.micro_size, None);
            commands.entity(entity).insert(CrewText);
            readouts::at(commands, root, r, single.id, Readout::Crew);
        }
        if let (Some(name), Some(r)) = (layout.name("infoSingleCrewDot"), layout.at(origin, "infoSingleCrewDotArea")) {
            let colour = style.color(&extra.race, Some(ratio));
            commands.entity(root).with_child((node(r), readouts::bind(single.id, Readout::Crew), GuiImage::new(name).with_color(colour)));
        }
    }
}
