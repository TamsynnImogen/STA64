//! Live readouts for the stock medium and tall single-selection cards.

use bevy::{picking::Pickable, prelude::*};
use sta_sim::{EntityId, Fixed, UnitView};

use super::{
    StockState,
    a1::info::{LiveTint, emblem_names},
    info::{node, text},
    layout::StockLayout,
};
use crate::ui::{
    gui_sprites::{GuiImage, GuiImageSampling},
    hud::{
        model::{self, HudFeed, Single},
        readouts::{self, Readout},
    },
    sprite_font::SpriteText,
    theme::Theme,
};

#[derive(Component, Clone, Copy)]
pub(in crate::ui::hud) struct SelectionReadout(EntityId, Field);

#[derive(Clone, Copy)]
enum Field {
    Crew,
    Officers,
    Energy,
}

impl Field {
    fn keys(self, builder: bool) -> (&'static str, &'static str, &'static str, Readout) {
        // ShipDisplay uses a different set of rectangles for the yard card.
        // Both modes use the same sprite identifiers and value sources (Armada II, static).
        match self {
            Self::Crew => {
                let (value, icon) = if builder {
                    ("infoBuildCrew", "infoBuildCrewIcon")
                } else {
                    ("infoCrewNum", "infoCrewNumIcon")
                };
                (value, icon, "crew_amt", Readout::Crew)
            }
            Self::Officers => {
                let (value, icon) = if builder {
                    ("infoBuildOfficers", "infoBuildOfficersIcon")
                } else {
                    ("infoOfficers", "infoOfficersIcon")
                };
                (value, icon, "officers", Readout::Officers)
            }
            Self::Energy => {
                let (value, icon) = if builder {
                    ("infoBuildEnergy", "infoBuildEnergyIcon")
                } else {
                    ("infoEnergy", "infoEnergyIcon")
                };
                (value, icon, "energy_amt", Readout::Energy)
            }
        }
    }

    fn value(self, unit: &UnitView, def: &sta_content::UnitDef) -> Option<(i64, f32)> {
        let whole = |value: Fixed| value.to_bits() / Fixed::ONE.to_bits();
        match self {
            Self::Crew => unit.crew.filter(|[_, max]| *max > Fixed::ZERO).map(|pool| {
                let count = whole(pool[0]).max(i64::from(pool[0] > Fixed::ZERO));
                (count, model::frac(pool))
            }),
            // The original reads the class's raw officer cost, independent of player capacity.
            Self::Officers => {
                (def.build.officers > Fixed::ZERO).then(|| (whole(def.build.officers), 1.0))
            }
            Self::Energy => unit
                .energy
                .filter(|[_, max]| *max > Fixed::ZERO)
                .map(|pool| {
                    let [now, max] = pool.map(|value| i128::from(value.to_bits()));
                    let percent = (now.clamp(0, max) * 100 + max / 2) / max;
                    (percent as i64, model::frac(pool))
                }),
        }
    }
}

pub(super) fn spawn(
    commands: &mut Commands,
    root: Entity,
    layout: &StockLayout,
    origin: Vec2,
    theme: &Theme,
    single: &Single,
    race: &str,
) {
    let id = single.id;
    let builder = single.has_build_queue;
    // Roots-hosted mods can use these ShipDisplay fields even with FleetOps presentation.
    for field in [Field::Crew, Field::Officers, Field::Energy] {
        let (value_key, icon_key, sprite, tip) = field.keys(builder);
        let marker = SelectionReadout(id, field);
        if let Some(rect) = layout.at(origin, value_key) {
            let label = text(
                commands,
                root,
                rect,
                String::new(),
                Color::WHITE,
                theme.small_size,
                None,
            );
            commands
                .entity(label)
                .insert((marker, Pickable::IGNORE, Visibility::Hidden));
            commands.entity(root).with_child((
                node(rect),
                marker,
                readouts::bind(id, tip),
                Visibility::Hidden,
            ));
        }
        if let Some(rect) = layout.at(origin, icon_key) {
            commands.entity(root).with_child((
                node(rect),
                marker,
                GuiImage::new(sprite),
                readouts::bind(id, tip),
                Visibility::Hidden,
            ));
        }
    }

    if !builder || !layout.has_a2_info() {
        return;
    }
    // Both the team tint and the emblem's sprite name come from the selected object.
    // The original only draws a backing when the mounted CFG names one.
    if let Some(sprite) = layout.name("infoSingleRaceIconBackground")
        && let Some(rect) = layout.at(origin, "infoSingleRaceIconArea")
    {
        commands.entity(root).with_child((
            node(rect),
            GuiImage::new(sprite),
            LiveTint::Race(id),
            Pickable::IGNORE,
        ));
    }
    if let Some(rect) = layout.at(origin, "infoSingleRaceIconDisplayArea") {
        let mut image = GuiImage::new("").with_sampling(GuiImageSampling::Linear);
        image.names = emblem_names(race);
        commands
            .entity(root)
            .with_child((node(rect), image, LiveTint::Race(id), Pickable::IGNORE));
    }
}

#[allow(clippy::type_complexity)]
pub(in crate::ui::hud) fn update(
    feed: Res<HudFeed>,
    state: Res<StockState>,
    mut parts: Query<(
        &SelectionReadout,
        &mut Visibility,
        Option<&mut SpriteText>,
        Option<&mut GuiImage>,
    )>,
) {
    for (&SelectionReadout(id, field), mut visibility, label, image) in &mut parts {
        let value = feed
            .view
            .unit(id)
            .and_then(|unit| feed.def(unit).map(|def| (unit, def)))
            .and_then(|(unit, def)| {
                field.value(unit, def).map(|(value, share)| {
                    // TextAndSprite uses the faction's thresholds for crew and energy.
                    let colour = if share > 0.0 {
                        state.info_style.color(&def.race, Some(share))
                    } else {
                        Color::srgb(0.2, 0.2, 0.2)
                    };
                    (value, colour)
                })
            });
        let next = if value.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != next {
            *visibility = next;
        }
        let Some((value, colour)) = value else {
            continue;
        };
        if let Some(mut label) = label {
            let value = match field {
                // Armada II's EnergyText prints a rounded percentage.
                Field::Energy => format!("{value}%"),
                _ => value.to_string(),
            };
            if label.text != value {
                label.text = value;
            }
            if label.color != colour {
                label.color = colour;
            }
        }
        if let Some(mut image) = image
            && image.color != colour
        {
            image.color = colour;
        }
    }
}
