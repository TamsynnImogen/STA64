//! Capabilities shared by every live, owned unit in the selection.
//! Cell positions in `model` are a fallback until mod GUI command layouts
//! are available; these rules describe sim support, not native grid parity.

use sta_content::ContentDb;
use sta_sim::specials::{SpecialKind, specials_of};
use sta_sim::view::UnitFlags;
use sta_sim::world::EntityKind;
use sta_sim::{Fixed, UnitView};

use super::model::{HudFeed, finished};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Capabilities {
    pub stop: bool,
    pub move_order: bool,
    pub attack: bool,
    pub build: bool,
    pub mine: bool,
    /// Same supported special kind at the same sim slot on every unit.
    pub specials: Vec<(u8, SpecialKind)>,
}

fn one(db: &ContentDb, u: &UnitView) -> Capabilities {
    let Some(def) = db.unit(u.def) else {
        return Capabilities::default();
    };
    let kind = EntityKind::of_def(def);
    let stop = matches!(kind, EntityKind::Craft | EntityKind::Station);
    if !finished(u) {
        return Capabilities {
            stop,
            ..Default::default()
        };
    }
    let mobile = kind == EntityKind::Craft
        && def.movement.max_speed > Fixed::ZERO
        && !u.flags.contains(UnitFlags::ENGINES_OFF);
    let attack = !u.flags.contains(UnitFlags::WEAPONS_OFF)
        && def.hardpoints.iter().any(|h| {
            db.weapon(h.weapon.key)
                .is_some_and(|w| !sta_sim::specials::is_special(w))
        });
    let specials = specials_of(db, def)
        .enumerate()
        .filter_map(|(slot, (kind, _weapon))| {
            (slot <= u8::MAX as usize && kind == SpecialKind::Cloak).then_some((slot as u8, kind))
        })
        .collect();
    Capabilities {
        stop,
        move_order: mobile,
        attack,
        build: !def.build.list.is_empty(),
        mine: mobile && (def.miner || def.class.0 == "freighter"),
        specials,
    }
}

pub fn shared(feed: &HudFeed) -> Capabilities {
    let Some(db) = feed.content.as_deref() else {
        return Capabilities::default();
    };
    let mut own = feed
        .selected_units()
        .into_iter()
        .filter(|u| u.owner == feed.player);
    let Some(first) = own.next() else {
        return Capabilities::default();
    };
    let mut common = one(db, first);
    for u in own {
        let next = one(db, u);
        common.stop &= next.stop;
        common.move_order &= next.move_order;
        common.attack &= next.attack;
        common.build &= next.build;
        common.mine &= next.mine;
        common.specials.retain(|s| next.specials.contains(s));
    }
    common
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::hud::model::{Act, Page, command, grid};
    use sta_content::{DefRef, Hardpoint, WeaponDef, WeaponKind};
    use sta_sim::command::{CommandKind, Order, Target};
    use sta_sim::{EntityId, PlayerId, Vec3, WorldView};
    use std::sync::Arc;

    #[test]
    fn specials_require_same_supported_slot_and_weapon_capability() {
        let fake = sta_content::fake::db();
        let mut build = ContentDb::builder()
            .profile(fake.profile())
            .features(fake.features().clone());
        for w in fake.weapons() {
            build = build.weapon(w.clone());
        }
        build = build.weapon(WeaponDef::new(
            "test_cloak",
            "cloakingdevice",
            WeaponKind::Beam,
        ));
        build = build.weapon(WeaponDef::new(
            "test_cloak_other",
            "cloakingdeviceimp",
            WeaponKind::Beam,
        ));
        build = build.weapon(WeaponDef::new(
            "test_unknown",
            "test_special",
            WeaponKind::Beam,
        ));
        for u in fake.units() {
            let mut u = u.clone();
            if u.odf == "fx_cruiser" || u.odf == "fx_frigate" {
                u.hardpoints.push(Hardpoint {
                    weapon: DefRef::new(if u.odf == "fx_cruiser" {
                        "test_cloak"
                    } else {
                        "test_cloak_other"
                    }),
                    node: "cloak".into(),
                    arc: None,
                });
            }
            if u.odf == "fx_frigate" {
                u.hardpoints.push(Hardpoint {
                    weapon: DefRef::new("test_unknown"),
                    node: "unknown".into(),
                    arc: None,
                });
            }
            build = build.unit(u);
        }
        let db = Arc::new(build.build().unwrap());
        let make = |id, odf| UnitView {
            id: EntityId(id),
            def: db.key(odf).unwrap(),
            owner: PlayerId(1),
            pos: Vec3::ZERO,
            heading: (Fixed::ZERO, Fixed::ONE),
            speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2],
            shields: [[Fixed::ZERO; 2]; 4],
            flags: UnitFlags::SELECTABLE,
            progress: None,
        };
        let mut feed = HudFeed {
            view: Arc::new(WorldView {
                units: vec![make(1, "fx_cruiser"), make(2, "fx_frigate")],
                ..Default::default()
            }),
            player: PlayerId(1),
            selected: vec![EntityId(1)],
            content: Some(db),
        };
        assert_eq!(shared(&feed).specials, vec![(0, SpecialKind::Cloak)]);
        assert!(
            grid(&feed, Page::Top)
                .iter()
                .any(|b| b.act == Act::Special(0))
        );
        assert!(matches!(
            command(&feed, &Act::Special(0)),
            Some((
                CommandKind::Order(Order::Special {
                    slot: 0,
                    target: Target::None,
                    ..
                }),
                _
            ))
        ));
        feed.selected.push(EntityId(2));
        assert_eq!(shared(&feed).specials, vec![(0, SpecialKind::Cloak)]);
        Arc::make_mut(&mut feed.view).units[1].flags =
            UnitFlags::SELECTABLE.with(UnitFlags::WEAPONS_OFF, true);
        assert!(!shared(&feed).attack);
    }
}
