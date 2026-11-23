//! Fleet Operations weapon pictures over the single-selection systems backdrop.

use bevy::prelude::*;
use sta_content::{Availability, ContentDb};
use sta_sim::{ContentKey, EntityId, Fixed, UnitView};

use super::super::{HudFeed, model::tech_state};
use super::node;
use crate::ui::{gui_sprites::GuiImage, notices::Tooltip};

/// The native consecutive weapon reader at RVA 0x10c790 stops at the first
/// absent `weaponN`; its UI has 32 slots (RVA 0x1ee05c). Missing icon positions
/// or unresolved weapons do not change later slots' one-based ODF ordinals.
const WEAPON_SLOTS: usize = 32;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::hud) struct WeaponIcon {
    pub slot: usize,
    backdrop: Entity,
    unit: EntityId,
    weapon: ContentKey,
}

// FleetOpsHook ShipSystemIcon_Render (0x1ed458) follows these RGB globals
// back to ArmadaL's ST3D_Colour initializers at 0x61ffb0, 0x61fff0 and
// 0x620070. The depleted underlay is literal RGB in the DLL at 0x1ed773.
pub(super) const READY: Color = Color::srgb(0.0, 1.0, 0.0);
const NEGATIVE: Color = Color::srgb(1.0, 1.0, 0.0);
const TECH_LOCKED: Color = Color::srgb(0.5, 0.5, 0.5);

fn weapon_tint(locked: bool, energy: Fixed, cost: Fixed) -> Color {
    if locked {
        TECH_LOCKED
    } else if cost <= Fixed::ZERO || energy >= cost {
        READY
    } else if energy < Fixed::ZERO {
        NEGATIVE
    } else {
        // STA64 tints one image. Blend the native foreground and underlay
        // in proportion to the charge until there is energy for a full shot.
        let charge = energy.to_f32() / cost.to_f32();
        Color::srgb(0.25 * (1.0 - charge), 0.2 + 0.8 * charge, 0.0)
    }
}

fn icon_position(raw: &sta_odf::Odf, slot: usize) -> Option<Vec2> {
    let key = format!("weapon{slot}IconPos");
    if !has_key(raw, &key) {
        return None;
    }
    let value = raw.str(&key)?;
    let mut words = value
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|word| !word.is_empty());
    let x = words.next()?.parse::<f32>().ok()?;
    let y = words.next()?.parse::<f32>().ok()?;
    (x.is_finite() && y.is_finite()).then_some(Vec2::new(x, y))
}

fn has_key(raw: &sta_odf::Odf, key: &str) -> bool {
    // PrefixFirst ODF lookup would otherwise let `weapon1` match `weapon10`.
    raw.keys()
        .any(|authored| authored.eq_ignore_ascii_case(key))
}

fn icon_rect(
    display: super::super::layout::R,
    size: super::super::layout::R,
    position: Vec2,
) -> super::super::layout::R {
    // Native RVA 0x1ed5c0–0x1ed658 scales both coordinates as percentages of
    // infoSingleSystemsDisplay, then centres infoSingleSystemsIcon at that point.
    super::super::layout::R::new(
        display.x + display.w * position.x * 0.01 - size.w * 0.5,
        display.y + display.h * position.y * 0.01 - size.h * 0.5,
        size.w,
        size.h,
    )
}

fn weapon_stem(name: &str) -> &str {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file.strip_suffix(".odf").unwrap_or(file)
}

pub(super) fn spawn(
    commands: &mut Commands,
    root: Entity,
    (display, size): (super::super::layout::R, super::super::layout::R),
    backdrop: Entity,
    unit: &UnitView,
    db: &ContentDb,
) {
    let Some(def) = db.unit(unit.def) else { return };

    for slot in 1..=WEAPON_SLOTS {
        let key = format!("weapon{slot}");
        if !has_key(&def.raw, &key) {
            break;
        }
        let Some(name) = def.raw.str(&key) else { break };
        let name = name.trim();
        if name.is_empty() {
            break;
        }
        let Some(position) = icon_position(&def.raw, slot) else {
            continue;
        };
        let Some(weapon) = db.key(weapon_stem(name)).and_then(|key| db.weapon(key)) else {
            continue;
        };
        let stem = weapon_stem(&weapon.odf);
        let mut image = GuiImage::new(format!("i_{stem}")).with_color(READY);
        // The pinned DLL at RVA 0x10f2fc names this ordered fallback.
        image.names.push("systemicon_default".to_owned());
        let tip = weapon.raw.str("tooltip").unwrap_or(&weapon.odf);
        let mut tooltip = Tooltip::new(crate::ui::i18n::tooltip_key(tip));
        if let Some(verbose) = weapon
            .raw
            .str("verboseTooltip")
            .or_else(|| weapon.raw.str("verbose"))
        {
            tooltip = tooltip.verbose(crate::ui::i18n::tooltip_key(verbose));
        }
        commands.spawn((
            node(icon_rect(display, size, position)),
            image,
            tooltip,
            WeaponIcon {
                slot,
                backdrop,
                unit: unit.id,
                weapon: weapon.key,
            },
            Visibility::Hidden,
            ChildOf(root),
        ));
    }
}

pub(in crate::ui::hud) fn update_tint(
    feed: Res<HudFeed>,
    mut icons: Query<(&WeaponIcon, &mut GuiImage)>,
) {
    // Only the single-selection card owns these sprites.
    let Some(unit) = icons
        .iter()
        .next()
        .and_then(|(icon, _)| feed.view.unit(icon.unit))
    else {
        return;
    };
    let Some(db) = feed.content.as_deref() else {
        return;
    };
    let Some(def) = db.unit(unit.def) else { return };
    let energy = unit.energy.map_or(def.special_energy_max, |[now, _]| now);
    let tech = db
        .tech_tree()
        .map(|tree| (tree, tech_state(&feed, db, &def.race, unit.owner)));
    for (icon, mut image) in &mut icons {
        let Some(weapon) = db.weapon(icon.weapon) else {
            continue;
        };
        let locked = weapon.base.special == Some(true)
            && tech.as_ref().is_some_and(|(tree, owned)| {
                matches!(
                    tree.availability(&weapon.odf, owned),
                    Availability::Locked { .. } | Availability::Withheld
                )
            });
        let color = weapon_tint(locked, energy, weapon.energy_cost);
        if image.color != color {
            image.color = color;
        }
    }
}

/// The GUI painter removes `ImageNode` when the mounted backdrop SPR or its
/// texture cannot be resolved. An unresolved icon also loses its hover target.
pub(in crate::ui::hud) fn update_visibility(
    pictures: Query<(), With<ImageNode>>,
    mut icons: Query<(Entity, &WeaponIcon, &mut Visibility)>,
) {
    for (entity, icon, mut visibility) in &mut icons {
        let next = if pictures.contains(icon.backdrop) && pictures.contains(entity) {
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
    use crate::ui::hud::layout::R;

    #[test]
    fn weapon_tints_follow_native_palette_with_live_charge() {
        let f = Fixed::from_int;
        assert_eq!(weapon_tint(false, f(0), f(0)), Color::srgb(0.0, 1.0, 0.0));
        assert_eq!(weapon_tint(false, f(10), f(10)), Color::srgb(0.0, 1.0, 0.0));
        assert_eq!(weapon_tint(false, f(0), f(10)), Color::srgb(0.25, 0.2, 0.0));
        assert_eq!(
            weapon_tint(false, f(5), f(10)),
            Color::srgb(0.125, 0.6, 0.0)
        );
        assert_eq!(weapon_tint(false, f(-1), f(10)), Color::srgb(1.0, 1.0, 0.0));
        assert_eq!(weapon_tint(true, f(10), f(10)), Color::srgb(0.5, 0.5, 0.5));
    }

    #[test]
    fn sparse_positions_keep_odf_slot_and_both_percentage_axes() {
        let raw = sta_odf::resolve("ship", &mut |_| Some(
            "weapon1 = missing\nweapon2 = beam\nweapon2IconPos = 61 50\nweapon3 = burst\nweapon3IconPos = -5, 125\n".to_owned()
        )).unwrap();
        assert_eq!(icon_position(&raw, 1), None);
        assert_eq!(icon_position(&raw, 2), Some(Vec2::new(61.0, 50.0)));
        assert_eq!(icon_position(&raw, 3), Some(Vec2::new(-5.0, 125.0)));
        assert_eq!(
            icon_rect(
                R::new(10.0, 20.0, 200.0, 100.0),
                R::new(0.0, 0.0, 34.0, 34.0),
                Vec2::new(61.0, 50.0)
            ),
            R::new(115.0, 53.0, 34.0, 34.0)
        );
    }
}
