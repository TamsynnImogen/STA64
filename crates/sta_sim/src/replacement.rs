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
            return key.filter(|&key| db.unit(key).is_some())
                .filter(|&key| crate::tech::replacement_allowed(db, &self.units, &self.players, &self.ai_seats,
                    id, key, true, options.ignore_fleet_cap, options.keep_owner, false).1)
                .map(|key| (key, [Fixed::ZERO; 3])).into_iter().collect();
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
            let (available, capacity) = crate::tech::replacement_allowed(
                db, &self.units, &self.players, &self.ai_seats, id, key,
                options.ignore_tech_tree || options.warp_in, options.ignore_fleet_cap, options.keep_owner,
                rule.mode == 1,
            );
            // The DLL's mode 1 explicitly reverses the ordinary tech test.
            if !capacity || (rule.mode == 1) == available {
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
            && let Some(cap) = self.officer_limit(source.owner)
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
            // Each result joins the census before the next one is admitted.
            // Retain this mode's tech test (including mode 1's reversal), so
            // an XML alternative must still satisfy tech and cap together.
            let (available, capacity) = crate::tech::replacement_allowed(
                db, &self.units, &self.players, &self.ai_seats, id, key,
                matches!(rule.mode, 3 | 4) || options.ignore_tech_tree || options.warp_in,
                options.ignore_fleet_cap, options.keep_owner, rule.mode == 1,
            );
            if !capacity || (rule.mode == 1) == available {
                continue;
            }
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
            let new_vessel = self.units.get(new).is_some_and(|unit| {
                matches!(unit.kind, EntityKind::Craft | EntityKind::Station)
            });
            if new_vessel && let Some(race) = &race_override {
                self.attrs.set_race(new, race);
            }
            if options.transfer_stats {
                if matches!(source.kind, EntityKind::Craft | EntityKind::Station) && new_vessel {
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
    use sta_content::{CapacityList, DefRef, Hardpoint, MapDef, TechTree, UnitDef, WeaponKind};
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

    fn capped_fixture(odf: &str, source_in_cap: bool) -> (Sim, Arc<ContentDb>, ContentKey, ContentKey) {
        capped_fixture_with_tree(odf, source_in_cap, "replace_base 0\nreplace_result 0\n")
    }

    fn capped_fixture_with_tree(odf: &str, source_in_cap: bool, tree_text: &str) -> (Sim, Arc<ContentDb>, ContentKey, ContentKey) {
        let mut base = UnitDef::new("replace_base", "craft");
        base.hull = Fixed::from_int(100);
        base.hardpoints.push(Hardpoint { weapon: DefRef::new("replace_test"), node: "hp01".into(), arc: None });
        let mut result = UnitDef::new("replace_result", "craft");
        result.hull = Fixed::from_int(100);
        result.fleet_cap = Some(Arc::new(CapacityList {
            name: "replace_cap".into(), max: 1,
            members: if source_in_cap { vec![("replace_base".into(), 1), ("replace_result".into(), 1)] }
                else { vec![("replace_result".into(), 1)] },
            ..Default::default()
        }));
        let raw = sta_odf::resolve("replace_test", &mut |_| Some(format!(
            "classLabel = replaceweapon\nreplacement0Class0 = replace_result\n{odf}"
        ))).unwrap();
        let mut weapon = WeaponDef::new("replace_test", "replaceweapon", WeaponKind::Beam);
        weapon.replace = sta_content::lifecycle::replace_def(&raw);
        let tree_name = if tree_text.trim_start().starts_with('<') { "replace.xml" } else { "replace.tt" };
        let (tree, warnings) = TechTree::parse(tree_name, tree_text);
        assert!(warnings.is_empty());
        let db = Arc::new(ContentDb::builder().unit(base).unit(result).weapon(weapon).tech_tree(tree).build().unwrap());
        let old = db.key("replace_base").unwrap();
        let new = db.key("replace_result").unwrap();
        let sim = Sim::from_setup(&GameSetup {
            seed: 7, content: db.clone(), map: Arc::new(MapDef::empty()),
            players: vec![
                PlayerSetup { id: OWNER, team: TeamId(1), race: "test".into(), control: Control::Human },
                PlayerSetup { id: PlayerId(2), team: TeamId(2), race: "test".into(), control: Control::Ai("test".into()) },
            ],
            options: GameOptions::default(),
        });
        (sim, db, old, new)
    }

    #[test]
    fn replacement_projects_retired_source_but_counts_kept_parent_against_cap() {
        let (mut sim, db, old, new) = capped_fixture("", true);
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        sim.step(&[use_weapon(source, Target::None)]);
        assert!(!sim.units.contains(source));
        assert_eq!(sim.units.iter().filter(|u| u.def == new).count(), 1);
        assert_eq!(Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap().digest(), sim.digest());

        let (mut sim, _, old, new) = capped_fixture("replacementKeepOwner = 1\n", true);
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        sim.step(&[use_weapon(source, Target::None)]);
        assert!(sim.units.contains(source));
        assert!(!sim.units.iter().any(|u| u.def == new));
    }

    #[test]
    fn replacement_ignore_fleet_cap_only_bypasses_capacity() {
        let (mut sim, db, old, new) = capped_fixture("replacementKeepOwner = 1\nreplacementIgnoreFleetCap = 1\n", true);
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        sim.step(&[use_weapon(source, Target::None)]);
        assert!(sim.units.contains(source));
        assert_eq!(sim.units.iter().filter(|u| u.def == new).count(), 1);
        assert_eq!(Sim::from_snapshot(&sim.snapshot(), Some(db)).unwrap().digest(), sim.digest());

        let (mut sim, _, old, new) = capped_fixture("replacementKeepOwner = 1\nignoreTechTree = 1\n", true);
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        sim.step(&[use_weapon(source, Target::None)]);
        assert!(sim.units.contains(source));
        assert!(!sim.units.iter().any(|u| u.def == new));

        let (mut sim, _, old, new) = capped_fixture_with_tree(
            "replacementIgnoreFleetCap = 1\n", false,
            "replace_base 0\nreplace_result 1 missing_prerequisite\n",
        );
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        sim.step(&[use_weapon(source, Target::None)]);
        assert!(sim.units.contains(source));
        assert!(!sim.units.iter().any(|u| u.def == new), "the cap flag does not bypass tech");
    }

    #[test]
    fn replacement_cap_counts_only_own_results_and_flag_changes_content_digest() {
        let (mut sim, _, old, new) = capped_fixture("", false);
        sim.spawn_def(new, PlayerId(2), Vec3::ZERO, HEADING).unwrap();
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        sim.step(&[use_weapon(source, Target::None)]);
        assert_eq!(sim.units.iter().filter(|u| u.def == new && u.owner == OWNER).count(), 1);

        let (mut full, _, old, new) = capped_fixture("", false);
        full.spawn_def(new, OWNER, Vec3::ZERO, HEADING).unwrap();
        let source = full.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        full.step(&[use_weapon(source, Target::None)]);
        assert!(full.units.contains(source));
        assert_eq!(full.units.iter().filter(|u| u.def == new && u.owner == OWNER).count(), 1);

        let (_, ordinary, _, _) = capped_fixture("", false);
        let (_, explicit_false, _, _) = capped_fixture("replacementIgnoreFleetCap = 0\n", false);
        let (_, bypass, _, _) = capped_fixture("replacementIgnoreFleetCap = 1\n", false);
        assert_eq!(ordinary.hash(), explicit_false.hash());
        assert_ne!(ordinary.hash(), bypass.hash());
    }

    #[test]
    fn multi_result_replacement_rechecks_capacity_after_each_spawn() {
        let (mut sim, _, old, new) = capped_fixture("replacement1Class0 = replace_result\n", false);
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        sim.step(&[use_weapon(source, Target::None)]);
        assert_eq!(sim.units.iter().filter(|u| u.def == new).count(), 1);
    }

    #[test]
    fn multi_result_replacement_keeps_xml_group_tech_and_cap_together() {
        let xml = r#"<techtree>
            <capacitylist name="replace_cap" maxCapValue="1"><obj>replace_result</obj></capacitylist>
            <capacitylist name="spare_cap" maxCapValue="1"><obj>other</obj></capacitylist>
            <object ODF="replace_base"/>
            <object ODF="replace_result">
                <requirement logicGroups="1"><obj>replace_base</obj><capList>replace_cap</capList></requirement>
                <requirement logicGroups="2"><obj>missing</obj><capList>spare_cap</capList></requirement>
            </object>
        </techtree>"#;
        let (mut sim, _, old, new) = capped_fixture_with_tree("replacement1Class0 = replace_result\n", false, xml);
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        sim.step(&[use_weapon(source, Target::None)]);
        assert!(!sim.units.contains(source));
        assert_eq!(sim.units.iter().filter(|u| u.def == new).count(), 1);
    }

    #[test]
    fn return_to_original_keeps_its_tech_bypass() {
        let (mut sim, _, old, _) = capped_fixture_with_tree(
            "replacementMode = 4\n", false, "replace_base -1\nreplace_result 0\n",
        );
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        sim.step(&[use_weapon(source, Target::None)]);
        assert!(!sim.units.contains(source));
        assert_eq!(sim.units.iter().filter(|u| u.def == old).count(), 1);
    }

    #[test]
    fn delayed_instant_replacement_rechecks_capacity_when_it_fires() {
        let odf = "replacementInstantPlayer = 1\nreplacementInstantDelay = 0.1\nDilithiumCost = 5\n";
        let (mut sim, _, old, new) = capped_fixture(odf, false);
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        sim.spawn_def(new, OWNER, Vec3::ZERO, HEADING).unwrap();
        let stock = sim.economy.stock_of(OWNER);
        for _ in 0..5 { sim.step(&[]); }
        assert!(sim.units.contains(source));
        assert_eq!(sim.units.iter().filter(|u| u.def == new).count(), 1);
        assert_eq!(sim.economy.stock_of(OWNER), stock);

        let (mut sim, _, old, new) = capped_fixture(&format!("{odf}replacementIgnoreFleetCap = 1\n"), false);
        let source = sim.spawn_def(old, OWNER, Vec3::ZERO, HEADING).unwrap();
        sim.spawn_def(new, OWNER, Vec3::ZERO, HEADING).unwrap();
        for _ in 0..5 { sim.step(&[]); }
        assert!(!sim.units.contains(source));
        assert_eq!(sim.units.iter().filter(|u| u.def == new).count(), 2);
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
