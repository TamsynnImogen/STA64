//! Capabilities shared by every live, owned unit in the selection.
//! Cell positions in `model` are a fallback until mod GUI command layouts
//! are available; these rules describe sim support, not native grid parity.

use sta_content::ContentDb;
use sta_sim::specials::{SpecialKind, specials_of};
use sta_sim::view::UnitFlags;
use sta_sim::world::EntityKind;
use sta_sim::{EntityId, Fixed, UnitView};

use super::model::{HudFeed, finished};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Capabilities {
    pub stop: bool,
    pub move_order: bool,
    pub attack: bool,
    pub build: bool,
    /// Stationary producers may set a launch rally; mobile builders may not.
    pub rally: bool,
    pub mine: bool,
    /// A pod colony ship the sim takes `Order::Colonize` for. The stock command
    /// (`colonize.odf`) names `colony_ship` as its source: the ship definition's
    /// `colony_ship` flag, plus the sim's own pod-ship rule (Borg beam ships are refused).
    /// Evidence: read from data (the game's command and ship definitions).
    pub colonize: bool,
    /// Same supported special kind at the same sim slot on every unit.
    pub specials: Vec<(u8, SpecialKind)>,
}

fn one(db: &ContentDb, u: &UnitView) -> Capabilities {
    let Some(def) = db.unit(u.def) else {
        return Capabilities::default();
    };
    let kind = EntityKind::of_def(def);
    let stop = matches!(kind, EntityKind::Craft | EntityKind::Station) || EntityKind::producer_def(def);
    if !finished(u) {
        return Capabilities {
            stop,
            ..Default::default()
        };
    }
    let mobile = kind == EntityKind::Craft
        && def.movement.max_speed > Fixed::ZERO
        && !u.flags.contains(UnitFlags::ENGINES_OFF)
        && !u.flags.contains(UnitFlags::BUILDING);
    // This is the resolved ODF's command-panel flag, not a raw sim rule.
    let attack = def.raw.bool("canAttack").unwrap_or(true)
        && !u.flags.contains(UnitFlags::WEAPONS_OFF)
        && def.hardpoints.iter().any(|h| {
            db.weapon(h.weapon.key)
                .is_some_and(|w| w.kind != sta_content::WeaponKind::PointDefence
                    && !w.raw.bool("special").unwrap_or(false)
                    && !sta_sim::specials::is_special(w))
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
        build: !def.build.list.is_empty() && !u.flags.contains(UnitFlags::BUILDING),
        rally: EntityKind::producer_def(def) && def.movement.max_speed == Fixed::ZERO
            && !def.build.list.is_empty() && !u.flags.contains(UnitFlags::BUILDING),
        mine: mobile && def.mines(),
        colonize: mobile && def.raw.bool("colony_ship").unwrap_or(false) && sta_sim::colony::is_pod_ship(db, def),
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
        common.rally &= next.rally;
        // Mine shows when any selected unit can mine; the others simply do not get the order (see `miners`).
        common.mine |= next.mine;
        // Likewise: only the colony ships of a mixed selection take a Colonize order (see `colony_ships`).
        common.colonize |= next.colonize;
        common.specials.retain(|s| next.specials.contains(s));
    }
    common
}

/// The members of `ids` that can take a Mine order (the rest of a mixed selection do not receive it).
pub fn miners(feed: &HudFeed, ids: &[EntityId]) -> Vec<EntityId> {
    let Some(db) = feed.content.as_deref() else { return Vec::new() };
    ids.iter().copied().filter(|id| feed.view.unit(*id).is_some_and(|u| one(db, u).mine)).collect()
}

/// The members of `ids` that can take a Colonize order (the rest of a mixed selection do not receive it).
pub fn colony_ships(feed: &HudFeed, ids: &[EntityId]) -> Vec<EntityId> {
    let Some(db) = feed.content.as_deref() else { return Vec::new() };
    ids.iter().copied().filter(|id| feed.view.unit(*id).is_some_and(|u| one(db, u).colonize)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::hud::model::{Act, Page, command, grid};
    use sta_content::{DefRef, Hardpoint, PlanetRole, UnitDef, WeaponDef, WeaponKind};
    use sta_sim::command::{CommandKind, Order, Target};
    use sta_sim::{EntityId, PlayerId, Vec3, WorldView};
    use std::sync::Arc;

    #[test]
    fn welding_constructor_keeps_stop_but_loses_move_mine_and_build() {
        let mut constructor = UnitDef::new("test_constructor", "constructionrig");
        constructor.movement = sta_content::MoveStats::from_f32(100.0, 1.0, 1.0, 1.0, 0.2);
        constructor.miner = true;
        constructor.build.list = vec![DefRef::new("test_station")];
        let db = ContentDb::builder().unit(constructor).unit(UnitDef::new("test_station", "starbase")).build().unwrap();
        let mut unit = UnitView {
            id: EntityId(1), def: db.key("test_constructor").unwrap(), owner: PlayerId(0),
            pos: Vec3::ZERO, heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4],
            crew: None, energy: None, autonomy: None,
            flags: UnitFlags::SELECTABLE, progress: None,
        };
        let idle = one(&db, &unit);
        assert!(idle.stop && idle.move_order && idle.mine && idle.build);
        unit.flags = unit.flags.with(UnitFlags::BUILDING, true);
        let welding = one(&db, &unit);
        assert!(welding.stop);
        assert!(!welding.move_order && !welding.mine && !welding.build);
    }

    #[test]
    fn a_freighter_with_mining_switched_off_cannot_mine() {
        let mut data = UnitDef::new("test_data", "freighter");
        data.movement = sta_content::MoveStats::from_f32(100.0, 1.0, 1.0, 1.0, 0.2);
        data.miner_off = true;
        let mut freight = UnitDef::new("test_freight", "freighter");
        freight.movement = data.movement;
        let db = ContentDb::builder().unit(data).unit(freight).build().unwrap();
        let unit = |odf: &str| UnitView {
            id: EntityId(1), def: db.key(odf).unwrap(), owner: PlayerId(0),
            pos: Vec3::ZERO, heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4],
            crew: None, energy: None, autonomy: None,
            flags: UnitFlags::SELECTABLE, progress: None,
        };
        assert!(!one(&db, &unit("test_data")).mine);
        assert!(one(&db, &unit("test_freight")).mine);
    }

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
            crew: None,
            energy: None, autonomy: None,
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

    #[test]
    fn typed_producer_builder_has_stop_and_build_while_background_has_neither() {
        let item = UnitDef::new("test_item", "craft");
        let mut producer = UnitDef::new("test_producer", "planet");
        producer.planet_role = Some(PlanetRole::Producer);
        producer.build.list = vec![DefRef::new("test_item")];
        let mut background = UnitDef::new("test_background", "planet_obj");
        background.planet_role = Some(PlanetRole::Background);
        let db = Arc::new(ContentDb::builder().unit(item).unit(producer).unit(background).build().unwrap());
        let make = |id, name| UnitView {
            id: EntityId(id), def: db.key(name).unwrap(), owner: PlayerId(0),
            pos: Vec3::ZERO, heading: (Fixed::ZERO, Fixed::ONE), speed: Fixed::ZERO,
            hull: [Fixed::ONE; 2], shields: [[Fixed::ZERO; 2]; 4],
            crew: None, energy: None, autonomy: None,
            flags: UnitFlags::SELECTABLE, progress: None,
        };
        let producer_view = make(1, "test_producer");
        let background_view = make(2, "test_background");
        let producer_caps = one(&db, &producer_view);
        assert!(producer_caps.stop && producer_caps.build);
        assert!(!producer_caps.move_order && !producer_caps.mine);
        let background_caps = one(&db, &background_view);
        assert!(!background_caps.stop && !background_caps.build);

        let mut feed = HudFeed {
            view: Arc::new(WorldView { units: vec![producer_view, background_view], ..Default::default() }),
            player: PlayerId(0), selected: vec![EntityId(1)], content: Some(db),
        };
        assert!(shared(&feed).stop && shared(&feed).build);
        feed.selected = vec![EntityId(2)];
        assert!(!shared(&feed).stop && !shared(&feed).build);
    }

    #[test]
    fn authored_attack_flags_and_producer_rally_match_available_commands() {
        let mut gun = WeaponDef::new("cmd_gun", "phaser", WeaponKind::Beam);
        gun.damage=Fixed::ONE;
        let pd=WeaponDef::new("cmd_pd", "pointdefenselaser", WeaponKind::PointDefence);
        let mut special=WeaponDef::new("cmd_special", "utility", WeaponKind::Beam);
        special.raw=Arc::new(sta_odf::resolve("w", &mut |_| Some("special=1".into())).unwrap());
        let make_def=|name:&str, weapon:Option<&str>, flag:Option<bool>| {
            let mut def=UnitDef::new(name,"craft");
            def.movement.max_speed=Fixed::ONE;
            if let Some(weapon)=weapon { def.hardpoints.push(Hardpoint {weapon:DefRef::new(weapon),node:"hp01".into(),arc:None}); }
            if let Some(flag)=flag {
                // The derived file's explicit zero must override inherited one.
                def.raw=Arc::new(sta_odf::resolve("child", &mut |n| Some(if n.trim_end_matches(".odf")=="base" {
                    "canAttack=1".into()
                } else { format!("#include \"base.odf\"\ncanAttack={}",u8::from(flag)) })).unwrap());
            }
            def
        };
        let normal=make_def("normal",Some("cmd_gun"),None);
        let disabled=make_def("disabled",Some("cmd_gun"),Some(false));
        let enabled=make_def("enabled",Some("cmd_gun"),Some(true));
        let pd_only=make_def("pd_only",Some("cmd_pd"),None);
        let special_only=make_def("special_only",Some("cmd_special"),None);
        let mut builder=make_def("builder",None,None);
        builder.build.list=vec![DefRef::new("normal")];
        let mut yard=UnitDef::new("yard","shipyard");
        yard.build.list=vec![DefRef::new("normal")];
        let db=ContentDb::builder().weapon(gun).weapon(pd).weapon(special)
            .unit(normal).unit(disabled).unit(enabled).unit(pd_only).unit(special_only).unit(builder).unit(yard).build().unwrap();
        let cap=|name| one(&db,&UnitView {
            id:EntityId(1),def:db.key(name).unwrap(),owner:PlayerId(0),pos:Vec3::ZERO,
            heading:(Fixed::ZERO,Fixed::ONE),speed:Fixed::ZERO,hull:[Fixed::ONE;2],
            shields:[[Fixed::ZERO;2];4],crew:None,energy: None, autonomy: None,flags:UnitFlags::SELECTABLE,progress:None,
        });
        assert!(cap("normal").attack && cap("enabled").attack);
        assert!(!cap("disabled").attack && !cap("pd_only").attack && !cap("special_only").attack);
        assert!(cap("builder").build && !cap("builder").rally);
        assert!(cap("yard").build && cap("yard").rally);
    }

}
