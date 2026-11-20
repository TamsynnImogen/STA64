//! Fleet Operations weapon pictures over the single-selection systems backdrop.

use bevy::prelude::*;
use sta_content::{ContentDb, UnitDef};

use super::{layout::StockLayout, node};
use crate::ui::{gui_sprites::GuiImage, notices::Tooltip};

/// The native consecutive weapon reader at RVA 0x10c790 stops at the first
/// absent `weaponN`; its UI has 32 slots (RVA 0x1ee05c). Missing icon positions
/// or unresolved weapons do not change later slots' one-based ODF ordinals.
const WEAPON_SLOTS: usize = 32;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::hud) struct WeaponIcon {
    pub slot: usize,
    backdrop: Entity,
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
    layout: &StockLayout,
    origin: Vec2,
    backdrop: Entity,
    def: &UnitDef,
    db: &ContentDb,
) {
    let (Some(display), Some(size)) = (
        layout.at(origin, "infoSingleSystemsDisplay"),
        layout.at(origin, "infoSingleSystemsIcon"),
    ) else {
        return;
    };

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
        let mut image = GuiImage::new(format!("i_{stem}"));
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
            WeaponIcon { slot, backdrop },
            Visibility::Hidden,
            ChildOf(root),
        ));
    }
}

/// The GUI painter removes `ImageNode` when the mounted backdrop SPR or its
/// texture cannot be resolved. Do not show icons over a transparent backdrop.
pub(in crate::ui::hud) fn update_visibility(
    pictures: Query<(), With<ImageNode>>,
    mut icons: Query<(&WeaponIcon, &mut Visibility)>,
) {
    for (icon, mut visibility) in &mut icons {
        let next = if pictures.contains(icon.backdrop) {
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
