//! The stock top InfoBar follows the actor under the pointer independently of selection.

use std::collections::BTreeMap;

use bevy::{picking::Pickable, prelude::*};
use sta_assets::a2fmt::odf::Odf;
use sta_content::{OdfIndex, Races};
use sta_sim::{Fixed, UnitView, view::UnitFlags};
use sta_vfs::Vfs;

use super::{StockState, layout::StockLayout};
use crate::input::hover::HoveredUnit;
use crate::ui::gui_sprites::GuiImage;
use crate::ui::hud::model::HudFeed;
use crate::ui::sprite_font::{SpriteText, sprite_text};
use crate::ui::theme::Theme;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::hud) enum InfoBarField {
    Name,
    Hull,
    Shields,
    Crew,
    Energy,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::hud) struct InfoBarIcon(pub InfoBarField);

impl InfoBarField {
    fn key(self) -> &'static str {
        match self {
            Self::Name => "shipName",
            Self::Hull => "hull",
            Self::Shields => "shields",
            Self::Crew => "crew_amt",
            Self::Energy => "energy_amt",
        }
    }

    fn all() -> [Self; 5] {
        [
            Self::Name,
            Self::Hull,
            Self::Shields,
            Self::Crew,
            Self::Energy,
        ]
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct InfoStyle {
    races: Races,
    thresholds: BTreeMap<String, (f32, f32)>,
}

impl InfoStyle {
    pub fn load(vfs: &Vfs) -> Self {
        let (index, _) = OdfIndex::build(vfs);
        let (races, _) = Races::load(vfs, &index);
        let mut thresholds = BTreeMap::new();
        for race in races.iter() {
            let name = format!("{}.odf", race.stem);
            let odf = Odf::load_layered(&name, |n, version| {
                index.load(vfs, n, version).map(String::into_bytes)
            });
            let (yellow, red) = odf.map_or((0.5, 0.25), |odf| {
                (
                    odf.get_f32("crewYellowStatus")
                        .filter(|v| v.is_finite())
                        .unwrap_or(0.5),
                    odf.get_f32("crewRedStatus")
                        .filter(|v| v.is_finite())
                        .unwrap_or(0.25),
                )
            });
            thresholds.insert(race.stem.clone(), (yellow, red));
        }
        Self { races, thresholds }
    }

    fn color(&self, race: &str, ratio: Option<f32>) -> Color {
        if ratio.is_none_or(|ratio| ratio < 0.0) {
            return Color::srgb(0.2, 0.2, 0.2);
        }
        let Some(race) = self.races.get(race) else {
            return Color::srgb(0.0, 1.0, 0.0);
        };
        let (yellow, red) = self
            .thresholds
            .get(&race.stem)
            .copied()
            .unwrap_or((0.5, 0.25));
        let Some(ratio) = ratio else {
            return Color::srgb(0.2, 0.2, 0.2);
        };
        if ratio > yellow {
            Color::srgb(0.0, 1.0, 0.0)
        } else if ratio > red {
            Color::srgb(1.0, 1.0, 0.0)
        } else {
            Color::srgb(1.0, 0.0, 0.0)
        }
    }
}

fn node(r: super::super::layout::R) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(r.x),
        top: Val::Px(r.y),
        width: Val::Px(r.w),
        height: Val::Px(r.h),
        ..default()
    }
}

pub(super) fn spawn(
    commands: &mut Commands,
    root: Entity,
    layout: &StockLayout,
    theme: &Theme,
    variant: usize,
) {
    if layout.info_area(variant).is_none() {
        return;
    }
    let origin = layout.info_origin(variant)
        + layout
            .point(&format!("infoBarOffset_{variant}"))
            .unwrap_or(Vec2::ZERO);
    for field in InfoBarField::all() {
        if let Some(rect) = layout.at(origin, field.key()) {
            let wrapper = commands
                .spawn((
                    Node {
                        justify_content: if field == InfoBarField::Name {
                            JustifyContent::FlexEnd
                        } else {
                            JustifyContent::FlexStart
                        },
                        align_items: AlignItems::Center,
                        ..node(rect)
                    },
                    Pickable::IGNORE,
                    ChildOf(root),
                ))
                .id();
            commands.spawn((
                field,
                Node::default(),
                sprite_text("", theme.small_size.max(18.0), theme.text),
                Visibility::Hidden,
                ChildOf(wrapper),
            ));
        }
        if field == InfoBarField::Name {
            continue;
        }
        let key = format!("icon_{}", field.key());
        if let Some(rect) = layout.at(origin, &key) {
            commands.spawn((
                InfoBarIcon(field),
                node(rect),
                GuiImage::new(field.key()).with_color(Color::WHITE),
                Visibility::Hidden,
                Pickable::IGNORE,
                ChildOf(root),
            ));
        }
    }
}

const SCALE: i128 = Fixed::ONE.to_bits() as i128;

fn percent(now: i128, max: i128) -> Option<i128> {
    (max > 0).then(|| (now * 200 + max).div_euclid(max * 2))
}

fn count(now: Fixed) -> i128 {
    let bits = i128::from(now.to_bits());
    if bits > 0 {
        (bits / SCALE).max(1)
    } else {
        bits / SCALE
    }
}

fn ratio(now: i128, max: i128) -> Option<f32> {
    (max > 0).then(|| (now as f64 / max as f64) as f32)
}

fn shield_bits(unit: &UnitView) -> (i128, i128) {
    unit.shields.iter().fold((0, 0), |(n, m), pair| {
        (
            n + i128::from(pair[0].to_bits()),
            m + i128::from(pair[1].to_bits()),
        )
    })
}

fn value(unit: &UnitView, field: InfoBarField) -> Option<(String, Option<f32>)> {
    let pair = match field {
        InfoBarField::Hull => {
            let (now, max) = (
                i128::from(unit.hull[0].to_bits()),
                i128::from(unit.hull[1].to_bits()),
            );
            return percent(now, max).map(|p| (p.to_string(), ratio(now, max)));
        }
        InfoBarField::Shields => {
            let (now, max) = shield_bits(unit);
            return percent(now, max).map(|p| (p.to_string(), ratio(now, max)));
        }
        InfoBarField::Crew => unit.crew?,
        InfoBarField::Energy => {
            let [now, max] = unit.energy?;
            let (now, max) = (i128::from(now.to_bits()), i128::from(max.to_bits()));
            return percent(now, max).map(|p| (p.to_string(), ratio(now, max)));
        }
        InfoBarField::Name => return None,
    };
    let (now, max) = (i128::from(pair[0].to_bits()), i128::from(pair[1].to_bits()));
    let status = if field == InfoBarField::Crew && max == 0 {
        Some(1.0)
    } else {
        ratio(now, max).map(|ratio| {
            if field == InfoBarField::Crew {
                ratio.min(1.0)
            } else {
                ratio
            }
        })
    };
    Some((count(pair[0]).to_string(), status))
}

// A2FOCraftIdentity's existing palette applies to percentage-based hull,
// shields and crew; special energy has its own optional fixed colour.
fn value_color(
    layout: Option<&StockLayout>,
    field: InfoBarField,
    style: &InfoStyle,
    race: &str,
    status: Option<f32>,
) -> Color {
    if let Some(layout) = layout {
        if let Some(color) = layout.colour(&format!("{}_color", field.key())) {
            return color;
        }
        let key = if field == InfoBarField::Energy {
            Some("specialEnergyIconColor")
        } else {
            status.filter(|ratio| ratio.is_finite()).map(|ratio| {
                if ratio <= 0.0 {
                    "systemIconDestroyedColor"
                } else if ratio > 0.5 {
                    "systemIconHealthyColor"
                } else if ratio > 0.25 {
                    "systemIconLowColor"
                } else {
                    "systemIconCriticalColor"
                }
            })
        };
        if let Some(color) = key.and_then(|key| layout.colour(key)) {
            return color;
        }
    }
    style.color(race, status)
}

#[allow(clippy::type_complexity)]
pub(in crate::ui::hud) fn update(
    feed: Res<HudFeed>,
    hover: Res<HoveredUnit>,
    state: Res<StockState>,
    theme: Res<Theme>,
    mut texts: Query<(&InfoBarField, &mut SpriteText, &mut Visibility), Without<InfoBarIcon>>,
    mut icons: Query<(&InfoBarIcon, &mut GuiImage, &mut Visibility), Without<InfoBarField>>,
) {
    let hovered = hover.0.and_then(|id| feed.view.unit(id)).filter(|unit| {
        unit.flags.contains(UnitFlags::SELECTABLE)
            && !unit.flags.contains(UnitFlags::DYING)
            && feed
                .view
                .visible_to(feed.player)
                .iter()
                .any(|visible| visible.id == unit.id)
    });
    let selected = (feed.selected.len() == 1)
        .then(|| feed.selected_units())
        .and_then(|units| units.first().copied())
        .filter(|unit| unit.flags.contains(UnitFlags::SELECTABLE))
        .filter(|unit| {
            feed.view
                .visible_to(feed.player)
                .iter()
                .any(|visible| visible.id == unit.id)
        });
    let unit = hovered.or(selected);
    let layout = state.layout.as_ref();
    for (field, mut text, mut visibility) in &mut texts {
        let shown = unit.and_then(|unit| {
            if *field == InfoBarField::Name {
                return Some((
                    if hovered.is_some() {
                        feed.unit_label(unit)
                    } else {
                        feed.name(unit)
                    },
                    None,
                ));
            }
            value(unit, *field)
        });
        let (label, status) = shown.unwrap_or_default();
        let color = if *field == InfoBarField::Name {
            layout
                .and_then(|layout| layout.colour("shipNameColor"))
                .unwrap_or(theme.text)
        } else {
            let race = unit
                .and_then(|unit| feed.content.as_deref().and_then(|db| db.unit(unit.def)))
                .map_or("", |def| def.race.as_str());
            value_color(layout, *field, &state.info_style, race, status)
        };
        if text.text != label {
            text.text = label.clone();
        }
        if text.color != color {
            text.color = color;
        }
        let next = if label.is_empty() {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != next {
            *visibility = next;
        }
    }
    for (icon, mut image, mut visibility) in &mut icons {
        let shown = unit.and_then(|unit| value(unit, icon.0));
        let color = if let Some((_, status)) = shown.as_ref() {
            let race = unit
                .and_then(|unit| feed.content.as_deref().and_then(|db| db.unit(unit.def)))
                .map_or("", |def| def.race.as_str());
            value_color(layout, icon.0, &state.info_style, race, *status)
        } else {
            Color::WHITE
        };
        if image.color != color {
            image.color = color;
        }
        let next = if shown.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != next {
            *visibility = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit() -> UnitView {
        sta_sim::world::Entity::new(
            sta_sim::world::EntityKind::Craft,
            sta_sim::ContentKey::NONE,
            sta_sim::PlayerId(0),
            sta_sim::Vec3::ZERO,
            (Fixed::ZERO, Fixed::ONE),
            sta_content::MoveStats::default(),
        )
        .view()
    }

    #[test]
    fn exact_percentage_and_count_formatting() {
        assert_eq!(percent(1, 2), Some(50));
        assert_eq!(percent(999, 1000), Some(100));
        assert_eq!(percent(0, 0), None);
        assert_eq!(count(Fixed::from_bits(1)), 1);
        assert_eq!(count(Fixed::from_int(73)), 73);
        assert_eq!(count(Fixed::from_bits(i64::MAX)), 2_147_483_647);
        let mut actor = unit();
        actor.hull = [Fixed::from_int(1), Fixed::from_int(2)];
        actor.shields = [[Fixed::from_int(1), Fixed::from_int(4)]; 4];
        actor.crew = Some([Fixed::from_int(73), Fixed::from_int(150)]);
        assert_eq!(value(&actor, InfoBarField::Hull).unwrap().0, "50");
        assert_eq!(value(&actor, InfoBarField::Shields).unwrap().0, "25");
        assert_eq!(value(&actor, InfoBarField::Crew).unwrap().0, "73");
        assert_eq!(value(&actor, InfoBarField::Energy), None);
        actor.crew = Some([Fixed::ZERO; 2]);
        assert_eq!(
            value(&actor, InfoBarField::Crew),
            Some(("0".into(), Some(1.0)))
        );
        actor.crew = None;
        assert_eq!(value(&actor, InfoBarField::Crew), None);
    }

    #[test]
    fn thresholds_use_strict_greater_than() {
        let mut style = InfoStyle::default();
        let race = sta_content::RaceDef {
            stem: "fx".into(),
            name: "fx".into(),
            display_key: None,
            slot: Some(0),
            interface: None,
            interface_sprites: None,
            single_player_music: None,
            multiplayer_music: None,
            standard_units: Vec::new(),
            minimal_units: Vec::new(),
            stock: [0; 4],
            lots_stock: [0; 4],
            normal_crew: None,
            lots_crew: None,
        };
        style.races = Races::from_defs(vec![race.clone()]);
        style.thresholds.insert("fx".into(), (0.5, 0.25));
        assert_eq!(style.color("fx", Some(0.5001)), Color::srgb(0.0, 1.0, 0.0));
        assert_eq!(style.color("fx", Some(0.5)), Color::srgb(1.0, 1.0, 0.0));
        assert_eq!(style.color("fx", Some(0.25)), Color::srgb(1.0, 0.0, 0.0));
        assert_eq!(
            style.color("missing", Some(0.0)),
            Color::srgb(0.0, 1.0, 0.0)
        );
    }

    #[test]
    fn authored_status_colour_is_available_to_override_the_status_fallback() {
        let (cfg, warnings) = sta_content::GuiCfg::parse(
            "infobar_test.cfg",
            "screenWidth = 1600\nscreenHeight = 1200\nhull_color = 0.2 0.4 0.6 1\n",
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        let layout = StockLayout::new(&cfg.layout_auto(), Vec2::new(1600.0, 1200.0));
        assert_eq!(
            layout.colour("hull_color"),
            Some(Color::srgba(0.2, 0.4, 0.6, 1.0))
        );
    }
    #[test]
    fn a2fo_existing_palette_commands_and_explicit_field_priority() {
        let cfg = sta_content::GuiCfg::parse("palette.cfg", "systemIconHealthyColor=0.2 1 0.2\nsystemIconLowColor=1 0.9 0\nsystemIconCriticalColor=1 0.15 0\nsystemIconDestroyedColor=1 0 1\nspecialEnergyIconColor=0.2 0.4 1\n").0.layout_auto();
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        let style = InfoStyle::default();
        for (ratio, expected) in [
            (1.0, Color::srgb(0.2, 1.0, 0.2)),
            (0.5, Color::srgb(1.0, 0.9, 0.0)),
            (0.25, Color::srgb(1.0, 0.15, 0.0)),
            (0.0, Color::srgb(1.0, 0.0, 1.0)),
        ] {
            for field in [
                InfoBarField::Hull,
                InfoBarField::Shields,
                InfoBarField::Crew,
            ] {
                assert_eq!(
                    value_color(Some(&layout), field, &style, "", Some(ratio)),
                    expected
                );
            }
        }
        assert_eq!(
            value_color(Some(&layout), InfoBarField::Energy, &style, "", Some(0.0)),
            Color::srgb(0.2, 0.4, 1.0)
        );
        let mut cfg = cfg;
        cfg.colours.insert(
            "hull_color".into(),
            sta_content::gui_cfg::Colour {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.4,
            },
        );
        let layout = StockLayout::new(&cfg, Vec2::new(1600.0, 1200.0));
        assert_eq!(
            value_color(Some(&layout), InfoBarField::Hull, &style, "", Some(1.0)),
            Color::srgba(0.0, 0.0, 0.0, 0.4)
        );
        let partial = sta_content::GuiCfg::parse("partial.cfg", "systemIconLowColor=1 0.9 0")
            .0
            .layout_auto();
        assert_eq!(
            value_color(
                Some(&StockLayout::new(&partial, Vec2::ONE)),
                InfoBarField::Hull,
                &style,
                "",
                Some(1.0)
            ),
            Color::srgb(0.0, 1.0, 0.0)
        );
    }
}
