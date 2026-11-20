//! Fleet Operations ReplaceWeapon. The ODF loader, choice rules and vessel
//! transfer are based on the clean FleetOpsHook reconstruction (static).

use serde::{Deserialize, Serialize};
use sta_content::{ContentDb, WeaponDef, lifecycle::{ReplaceClass, ReplaceDef}};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, Tick, Vec3};

use crate::{Sim, SimEvent, command::{Order, Target}, specials::{SpecialKind, specials_indexed}, view::UnitFlags};
use crate::world::{Entities, EntityKind, Fnv, Table};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Mount {
    ready: Tick,
    children: Vec<EntityId>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Actor {
    parent: Option<ContentKey>,
    original: Option<ContentKey>,
    mounts: Vec<(usize, Mount)>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Replacements(Table<Actor>);

#[derive(Serialize, Deserialize)]
pub(crate) struct Record(Vec<(EntityId, Actor)>);

impl Replacements {
    pub(crate) fn remove(&mut self, id: EntityId) {
        self.0.remove(id);
        for (_, actor) in self.0.iter_mut() {
            for (_, mount) in &mut actor.mounts {
                mount.children.retain(|&child| child != id);
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn record(&self) -> Record {
        Record(self.0.iter().map(|(id, actor)| (id, actor.clone())).collect())
    }

    pub(crate) fn restore(record: Record, units: &Entities) -> Option<Self> {
        if record.0.iter().any(|(id, actor)| {
            !units.contains(*id)
                || !actor.mounts.windows(2).all(|pair| pair[0].0 < pair[1].0)
                || actor.mounts.iter().any(|(_, mount)| {
                    !mount.children.windows(2).all(|pair| pair[0] < pair[1])
                        || mount.children.iter().any(|&child| !units.contains(child))
                })
        }) {
            return None;
        }
        Some(Self(Table::from_rows(record.0)?))
    }

    pub(crate) fn hash_into(&self, hash: &mut Fnv) {
        hash.str("replaceweapon");
        hash.u64(self.0.len() as u64);
        for (id, actor) in self.0.iter() {
            hash.u64(u64::from(id.0));
            hash.u64(actor.parent.map_or(0, |key| u64::from(key.0)));
            hash.u64(actor.original.map_or(0, |key| u64::from(key.0)));
            hash.u64(actor.mounts.len() as u64);
            for (index, mount) in &actor.mounts {
                hash.u64(*index as u64);
                hash.u64(mount.ready);
                hash.u64(mount.children.len() as u64);
                for child in &mount.children {
                    hash.u64(u64::from(child.0));
                }
            }
        }
    }

    fn mount(&self, id: EntityId, index: usize) -> Option<&Mount> {
        let actor = self.0.get(id)?;
        let at = actor.mounts.binary_search_by_key(&index, |row| row.0).ok()?;
        Some(&actor.mounts[at].1)
    }

    fn set_mount(&mut self, id: EntityId, index: usize, mount: Mount) {
        if self.0.get(id).is_none() {
            self.0.insert(id, Actor::default());
        }
        let mounts = &mut self.0.get_mut(id).expect("actor was inserted").mounts;
        match mounts.binary_search_by_key(&index, |row| row.0) {
            Ok(at) => mounts[at].1 = mount,
            Err(at) => mounts.insert(at, (index, mount)),
        }
    }
}

fn weighted<'a>(sim: &mut Sim, group: &'a [ReplaceClass]) -> Option<&'a ReplaceClass> {
    if group.len() <= 1 {
        return group.first();
    }
    let total = group.iter().fold(0i32, |sum, item| sum.wrapping_add(item.weight));
    let ticket = if total > 0 { (sim.rng.next_u64() % total as u64) as i32 + 1 } else { 1 };
    let mut cumulative = 0i32;
    group.iter().find(|item| {
        let previous = cumulative;
        cumulative = cumulative.wrapping_add(item.weight);
        previous < ticket && ticket <= cumulative
    }).or_else(|| group.first())
}

fn scaled(old: [Fixed; 2], new: [Fixed; 2]) -> Fixed {
    if old[1] > Fixed::ZERO {
        (old[0] / old[1] * new[1]).clamp(Fixed::ZERO, new[1])
    } else {
        new[0]
    }
}

impl Sim {
    pub(crate) fn order_replacements(
        &mut self, db: &ContentDb, player: PlayerId, ids: &[EntityId], slot: u8, target: Target,
    ) -> bool {
        // The selection can contain the same entity more than once on the wire.
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let mut accepted = false;
        for id in ids {
            let Some(source) = self.units.get(id).filter(|u| !player.is_player() || u.owner == player) else { continue };
            let Some(def) = db.unit(source.def) else { continue };
            let Some((index, SpecialKind::Replace, weapon)) = specials_indexed(db, def).nth(slot as usize) else { continue };
            accepted |= self.execute_replacement(db, id, index, weapon, target, true);
        }
        accepted
    }

    fn replacement_choices(
        &mut self, db: &ContentDb, id: EntityId, source: ContentKey, rule: &ReplaceDef, room: usize,
    ) -> Vec<(ContentKey, [Fixed; 3])> {
        let options = &rule.options;
        if matches!(rule.mode, 3 | 4) {
            let lineage = self.replacements.0.get(id);
            let key = if rule.mode == 3 {
                lineage.and_then(|actor| actor.parent)
            } else {
                Some(lineage.and_then(|actor| actor.original).unwrap_or(source))
            };
            return key.filter(|&key| db.unit(key).is_some()).map(|key| (key, [Fixed::ZERO; 3])).into_iter().collect();
        }
        let owner = self.units.get(id).expect("replacement source exists").owner;
        let race = self.attrs.player_race(owner)
            .or_else(|| db.unit(source).map(|def| def.race.as_str())).unwrap_or_default().to_owned();
        let source_name = db.name(source).unwrap_or_default();
        let selected: Vec<_> = match rule.mode {
            0 => rule.entries.iter().take(room).collect(),
            1 => rule.entries.iter().find(|group| group.condition.as_deref() == Some(race.as_str())).into_iter().collect(),
            2 => rule.entries.iter().find(|group| group.condition.as_deref() == Some(source_name)).into_iter().collect(),
            _ => Vec::new(),
        };
        let mut out = Vec::new();
        for group in selected {
            let Some(option) = weighted(self, &group.classes) else { continue };
            let Some(key) = db.key(&option.odf).filter(|&key| db.unit(key).is_some()) else { continue };
            if options.ignore_same_class && key == source {
                continue;
            }
            let available = options.ignore_tech_tree || options.warp_in
                || crate::tech::build_allowed(db, &self.units, &self.players, &self.ai_seats, id, key);
            // The DLL's mode 1 explicitly reverses the ordinary tech test.
            if (rule.mode == 1) == available {
                continue;
            }
            let offset = if rule.mode == 2 {
                group.classes.iter().position(|item| std::ptr::eq(item, option))
                    .and_then(|index| rule.entries.first()?.classes.get(index))
                    .map_or(option.offset, |first| first.offset)
            } else {
                option.offset
            };
            out.push((key, offset));
        }
        out
    }

    /// Fire a single mount. Preparation validates all costs before creating
    /// anything and commits energy/resources only if at least one ship spawns.
    pub(crate) fn execute_replacement(
        &mut self, db: &ContentDb, id: EntityId, index: usize, weapon: &WeaponDef,
        target: Target, manual: bool,
    ) -> bool {
        let Some(rule) = weapon.replace.as_ref() else { return false };
        let options = &rule.options;
        let Some(source) = self.units.get(id).filter(|u| u.hull[0] > Fixed::ZERO
            && !u.is_unfinished() && !u.flags.contains(UnitFlags::DYING)).cloned() else { return false };
        if manual && weapon.base.need_target == Some(true) && target == Target::None {
            return false;
        }
        let aim = match target {
            Target::Entity(target) => {
                let Some(unit) = self.units.get(target).filter(|u| u.hull[0] > Fixed::ZERO) else { return false };
                unit.pos
            }
            Target::Point(at) => at,
            Target::None => source.pos,
        };
        let mut mount = self.replacements.mount(id, index).cloned().unwrap_or_default();
        if manual && self.tick < mount.ready {
            return false;
        }
        mount.children.retain(|&child| self.units.get(child).is_some_and(|u| u.owner == source.owner));
        let room = if options.concurrent_children == 0 { usize::MAX }
            else { options.concurrent_children.saturating_sub(mount.children.len()) };
        if room == 0 {
            return false;
        }
        let Some(def) = db.unit(source.def) else { return false };
        if let Some(n) = rule.required_officers.filter(|&n| n != 0)
            && let Some(cap) = self.economy.officer_limit(source.owner)
        {
            let required = Fixed::from_int(n.clamp(i32::MIN as i64, i32::MAX as i64) as i32);
            let after = self.officer_census(source.owner) + required;
            if (n > 0 && cap > Fixed::ZERO && after > cap) || (n < 0 && after < cap) {
                return false;
            }
        }
        let energy_capacity = crate::specials::energy_def(def).map_or(Fixed::ZERO, |pool| pool.max);
        let current_energy = self.specials.energy_of(id).unwrap_or(energy_capacity);
        let energy = if manual { current_energy - weapon.energy_cost } else { current_energy };
        let stock = self.economy.stock_of(source.owner);
        let costs = self.economy.scaled_cost(source.owner, &options.cost);
        let crew = self.economy.crew_of(source.owner);
        if manual && (energy < Fixed::ZERO
            || costs.iter().zip(&stock).any(|(&cost, &balance)| cost > balance)
            || options.crew_cost > crew.unwrap_or(Fixed::ZERO))
        {
            return false;
        }
        let choices = self.replacement_choices(db, id, source.def, rule, room);
        if choices.is_empty() {
            return false;
        }
        let attack = self.weapons.attack_of(id);
        let course = self.movement.course(id).cloned();
        let name = options.keep_name.then(|| self.switches.names().into_iter()
            .find(|row| row.0 == id).map(|(_, name)| name)).flatten();
        let original = self.replacements.0.get(id).and_then(|actor| actor.original).unwrap_or(source.def);
        let race_override = self.attrs.race(id).map(str::to_owned);
        let mut spawned = Vec::new();
        for (key, [x, y, z]) in choices {
            let (hx, hz) = source.heading;
            let mut at = Vec3 { x: aim.x + hz * x + hx * z, y: aim.y + y, z: aim.z - hx * x + hz * z };
            if options.find_open_spot || options.warp_in {
                at = self.open_replacement_position(db, key, id, at);
            }
            let entry = if options.warp_in && (at.x != source.pos.x || at.z != source.pos.z) {
                self.open_replacement_position(db, key, id, source.pos)
            } else {
                at
            };
            let Some(new) = self.spawn_unqueued_def(key, source.owner, entry, source.heading) else { continue };
            self.replacements.0.insert(new, Actor { parent: Some(source.def), original: Some(original), mounts: Vec::new() });
            if let Some(race) = &race_override {
                self.attrs.set_race(new, race);
            }
            if options.transfer_stats {
                if matches!(source.kind, EntityKind::Craft | EntityKind::Station)
                    && self.units.get(new).is_some_and(|unit| matches!(unit.kind, EntityKind::Craft | EntityKind::Station))
                {
                    self.autonomy.copy_to(id, new);
                    self.edit_alert(new, self.alert_of(id));
                }
                if let Some(unit) = self.units.get_mut(new) {
                    unit.hull[0] = scaled(source.hull, unit.hull);
                    let old_shields = source.shields.view();
                    let faces = if unit.shields.directional { 4 } else { 1 };
                    for (face, old) in old_shields.iter().take(faces).enumerate() {
                        unit.shields.now[face] = scaled(*old, [unit.shields.now[face], unit.shields.max]);
                    }
                    if let (Some(old), Some(new)) = (source.crew, &mut unit.crew) {
                        new[0] = scaled(old, *new);
                    }
                }
                if !options.reset_energy && let Some(pool) = db.unit(key).and_then(crate::specials::energy_def) {
                    let transferred = scaled([energy, energy_capacity], [pool.max, pool.max]);
                    self.specials.set_energy(&self.units, db, new, transferred);
                }
            }
            if options.reset_crew && let Some(crew) = self.units.get_mut(new).and_then(|unit| unit.crew.as_mut()) {
                crew[0] = crew[1];
            }
            if let Some(name) = &name {
                self.apply_switch_edit(&crate::command::Edit::Name(new, name.clone()));
            }
            if entry != at && self.units.get(new).is_some_and(|u| u.kind == EntityKind::Craft) {
                self.edit_warp_speed(new, Some(options.warp_speed));
                self.apply_scripted(source.owner, &Order::MoveWarp { units: vec![new], to: at, queue: false });
            } else if options.maintain_orders && self.units.get(new).is_some_and(|u| u.kind == EntityKind::Craft) {
                if let Some(target) = attack.filter(|&target| target != id) {
                    self.weapons.order_attack(db, &self.units, source.owner, &[new], target);
                } else {
                    self.set_target(new, source.target);
                    if let Some(course) = &course {
                        self.movement.set_waypoints(new, course.waypoints.clone());
                    }
                }
            }
            spawned.push(new);
        }
        let Some(&last) = spawned.last() else { return false };
        if manual {
            for (slot, cost) in costs.into_iter().enumerate() {
                if cost != Fixed::ZERO { self.economy.spend(source.owner, slot as u8, cost); }
            }
            if options.crew_cost > Fixed::ZERO {
                self.economy.withdraw_crew(source.owner, options.crew_cost);
            } else if options.crew_cost < Fixed::ZERO {
                self.economy.edit_team_crew(source.owner, crew.unwrap_or(Fixed::ZERO) - options.crew_cost);
            }
            self.specials.set_energy(&self.units, db, id, energy);
        }
        if options.keep_owner {
            mount.ready = self.tick.saturating_add(weapon.refire);
            if options.concurrent_children > 0 {
                mount.children.extend(spawned.iter().copied());
            }
            self.replacements.set_mount(id, index, mount);
        } else {
            if self.units.get(last).is_some_and(|unit| unit.kind == EntityKind::Craft) {
                self.weapons.retarget_replacement_shots(id, last);
            }
            self.retire(id);
        }
        // Mode 0 transfers an existing selection to the first result. Mode 2
        // preserves the source selection wherever it survives.
        self.events.push(SimEvent::Replaced { old: id, new: spawned[0], select: options.select_mode != 2 });
        for new in spawned {
            self.push_build_items(new);
        }
        true
    }

    fn open_replacement_position(&self, db: &ContentDb, key: ContentKey, source: EntityId, origin: Vec3) -> Vec3 {
        let radius = db.unit(key).map_or(Fixed::ZERO, |def| def.radius).max(Fixed::ONE);
        let step = radius + radius;
        for ring in 0..=16 {
            let distance = Fixed::from_int(ring) * step;
            for (dx, dz) in [(0, 0), (1, 0), (0, 1), (-1, 0), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let at = Vec3 { x: origin.x + Fixed::from_int(dx) * distance, y: origin.y, z: origin.z + Fixed::from_int(dz) * distance };
                if self.units.iter().filter(|u| u.id != source).all(|u| {
                    let other = db.unit(u.def).map_or(Fixed::ZERO, |def| def.radius);
                    let x = (at.x - u.pos.x).abs();
                    let z = (at.z - u.pos.z).abs();
                    x.max(z) >= radius + other
                }) {
                    return at;
                }
            }
        }
        origin
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{WorldSource, command::{Command, CommandKind}, setup::{Control, GameOptions, GameSetup, PlayerSetup}};
    use sta_content::{DefRef, Hardpoint, MapDef, UnitDef, WeaponKind};
    use sta_core::TeamId;
    use std::sync::Arc;

    const OWNER: PlayerId = PlayerId(1);
    const HEADING: (Fixed, Fixed) = (Fixed::ONE, Fixed::ZERO);

    fn fixture(odf: &str) -> (Sim, Arc<ContentDb>, ContentKey, ContentKey) {
        let mut base = UnitDef::new("replace_base", "craft");
        base.hull = Fixed::from_int(100);
        base.special_energy_max = Fixed::from_int(100);
        base.hardpoints.push(Hardpoint { weapon: DefRef::new("replace_test"), node: "hp01".into(), arc: None });
        let mut result = UnitDef::new("replace_result", "craft");
        result.hull = Fixed::from_int(200);
        result.special_energy_max = Fixed::from_int(200);
        let mut weapon = WeaponDef::new("replace_test", "replaceweapon", WeaponKind::Beam);
        let raw = sta_odf::resolve("replace_test", &mut |_| Some(format!(
            "classLabel = replaceweapon\nreplacement0Class0 = replace_result\n{odf}"
        ))).unwrap();
        weapon.replace = sta_content::lifecycle::replace_def(&raw);
        weapon.energy_cost = Fixed::from_int(20);
        weapon.refire = 60;
        weapon.base.need_target = raw.bool("needTarget");
        let db = Arc::new(ContentDb::builder().unit(base).unit(result).weapon(weapon).build().unwrap());
        let old = db.key("replace_base").unwrap();
        let new = db.key("replace_result").unwrap();
        let sim = Sim::from_setup(&GameSetup {
            seed: 7, content: db.clone(), map: Arc::new(MapDef::empty()),
            players: vec![PlayerSetup { id: OWNER, team: TeamId(1), race: "test".into(), control: Control::Human }],
            options: GameOptions::default(),
        });
        (sim, db, old, new)
    }

    fn use_weapon(id: EntityId, target: Target) -> Command {
        Command { player: OWNER, seq: 0, kind: CommandKind::Order(Order::Special { units: vec![id], slot: 0, target }) }
    }

    #[test]
    fn manual_targeted_replacement_is_atomic_and_transfers_relative_stats() {
        let (mut sim, db, old, new) = fixture("needTarget = 1\nDilithiumCost = 25\nreplacement0Class0Position = 0, 0, 5\n");
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        sim.units.get_mut(source).unwrap().hull[0] = Fixed::from_int(50);
        sim.economy.set_stock(OWNER, [Fixed::ZERO; 4]);
        sim.step(&[use_weapon(source, Target::Point(Vec3::ZERO))]);
        assert!(sim.units.contains(source));
        assert!(!sim.units.iter().any(|u| u.def == new));
        sim.economy.set_stock(OWNER, [Fixed::from_int(30); 4]);
        sim.step(&[use_weapon(source, Target::None)]);
        assert_eq!(sim.economy.stock_of(OWNER)[0], Fixed::from_int(30));
        let aim = Vec3 { x: Fixed::from_int(80), y: Fixed::ZERO, z: Fixed::from_int(100) };
        let next = sim.step(&[use_weapon(source, Target::Point(aim))]);
        assert!(!sim.units.contains(source));
        let output = sim.units.iter().find(|u| u.def == new).unwrap();
        assert_eq!(output.pos.x, aim.x + Fixed::from_int(5));
        assert_eq!(output.hull[0], Fixed::from_int(100));
        let energy = sim.specials.energy_of(output.id).unwrap();
        assert!((energy - Fixed::from_int(160)).abs() < Fixed::from_f64(0.000_001));
        assert_eq!(sim.economy.stock_of(OWNER)[0], Fixed::from_int(5));
        assert!(next.events.iter().any(|e| matches!(e, SimEvent::Replaced { old, new, .. } if *old == source && *new == output.id)));
        assert_eq!(Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap().digest(), sim.digest());
    }

    #[test]
    fn autonomous_keep_owner_cycles_but_respects_live_child_capacity_after_load() {
        let (mut sim, db, old, new) = fixture(
            "replacementInstantPlayer = 1\nreplacementInstantCycle = 0.1\nreplacementKeepOwner = 1\nconcurrentChilds = 1\n"
        );
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        for _ in 0..8 {
            sim.step(&[]);
        }
        let child = sim.units.iter().find(|u| u.def == new).expect("instant cycle created a child").id;
        assert_eq!(sim.units.iter().filter(|u| u.def == new).count(), 1);
        let mut restored = Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap();
        assert!(sim.despawn(child, None) && restored.despawn(child, None));
        for _ in 0..6 {
            sim.step(&[]);
            restored.step(&[]);
            assert_eq!(restored.digest(), sim.digest());
        }
        assert!(sim.units.contains(source));
        assert_eq!(sim.units.iter().filter(|u| u.def == new).count(), 1);
    }

    #[test]
    fn fo4_nebula_retrofit_runs_pushbuild_then_replacement_when_installed() {
        let Ok(root) = std::env::var("STA64_REPLACE_FO4_INSTALL") else { return };
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some("FO4")).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let (db, _) = ContentDb::load(&vfs, &index).unwrap();
        let first = db.key("fed_nebula_engP").unwrap();
        let pushed = db.key("fed_nebula_engPC").unwrap();
        let result = db.key("fed_nebula_engPM").unwrap();
        let mut sim = Sim::from_setup(&GameSetup {
            seed: 8, content: Arc::new(db), map: Arc::new(MapDef::empty()),
            players: vec![PlayerSetup { id: OWNER, team: TeamId(1), race: "federation".into(), control: Control::Human }],
            options: GameOptions::default(),
        });
        sim.economy.set_stock(OWNER, [Fixed::from_int(90_000); 4]);
        sim.economy.edit_team_crew(OWNER, Fixed::from_int(9_000));
        let source = sim.spawn_def(first, OWNER, Vec3::ZERO, HEADING).unwrap();
        assert_eq!(sim.production.queue(source).first(), Some(&(pushed, 0)));
        let mut pushed_finished = false;
        let mut replaced = false;
        for _ in 0..2_200 {
            let out = sim.step(&[]);
            for event in &out.events {
                match *event {
                    SimEvent::Built { builder, .. } if builder == source => pushed_finished = true,
                    SimEvent::Replaced { old, new, .. } if old == source => {
                        assert_eq!(sim.units.get(new).map(|u| u.def), Some(result));
                        replaced = true;
                    }
                    _ => {}
                }
            }
            if replaced { break; }
        }
        assert!(pushed_finished, "the pushed build never completed");
        assert!(replaced, "the installed retrofit weapon never fired");
    }
}
