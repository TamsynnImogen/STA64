//! Start helpers and effects (SIM-28): the lifecycle of the hidden objects a
//! race's `standardUnits` place and of what they spawn.
//!
//! Rules, read from the typed content (`sta_content::lifecycle`; ODF keys
//! `autoExpireTime`, `lifeTimer`, `replacementInstant*`, `replacementMode`,
//! `replacement<i>class<j>`, `requiredOfficers`) and applied to `hidden = 1`
//! objects only (the only ones measured):
//!
//! - **Expiry.** An object with `autoExpireTime` (else `lifeTimer`) removes
//!   itself that many seconds after it appears.
//! - **Instant replacement.** A hardpoint weapon of class `replaceweapon` with
//!   `replacementInstantPlayer` / `AI` / `Derelict` set for the owner's kind
//!   fires `replacementInstantDelay` seconds after the object appears (the
//!   first such weapon in hardpoint order; the source is gone after it fires).
//!   Entries whose `replacement<i>condition` is absent or names the source
//!   ODF are candidates: mode 0 spawns the chosen class of every candidate,
//!   mode 2 of the first. An entry's class is drawn by `randomweight` from the
//!   sim generator, else its first class. Replacements appear at the source's
//!   position plus `replacement<i>Class<j>Position` (world axes), with its
//!   owner and heading, and run their own lifecycles. `requiredOfficers`
//!   needs the owner's officer maximum to reach it; with none set it does not
//!   fire. The source is removed when something was spawned.
//! - An owner is a *player* (human, remote, idle seat), *AI* (`Control::Ai`) or
//!   *derelict* (no owner).
//!
//! State: the pending rows, in the digest and snapshot once any exist.

use sta_content::lifecycle::{Owner, ReplaceDef};
use sta_content::{ContentDb, Warning};
use sta_core::{EntityId, Fixed, PlayerId, Tick, Vec3};

use crate::world::{Fnv, NEUTRAL};
use crate::Sim;

/// What a row does when due.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Act {
    Expire,
    /// Fire the replacement weapon at this hardpoint.
    Replace(u8),
}

/// A pending lifecycle step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Row {
    pub at: Tick,
    pub id: EntityId,
    pub act: Act,
}

/// Pending steps, ascending.
#[derive(Debug, Clone, Default)]
pub(crate) struct StartEffects {
    rows: Vec<Row>,
}

/// A snapshot row: `(tick, entity, act)`, with `act` 0 for expiry and
/// `1 + hardpoint` for a replacement.
pub(crate) type Rows = Vec<(u64, u32, u16)>;

impl StartEffects {
    fn add(&mut self, row: Row) {
        let at = self.rows.binary_search(&row).unwrap_or_else(|i| i);
        self.rows.insert(at, row);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn hash_into(&self, h: &mut Fnv) {
        h.str("start-effects-v1");
        h.u64(self.rows.len() as u64);
        for r in &self.rows {
            h.u64(r.at);
            h.u64(r.id.0 as u64);
            h.u64(match r.act {
                Act::Expire => 0,
                Act::Replace(i) => 1 + u64::from(i),
            });
        }
    }

    pub(crate) fn to_rec(&self) -> Rows {
        self.rows
            .iter()
            .map(|r| (r.at, r.id.0, match r.act { Act::Expire => 0, Act::Replace(i) => 1 + u16::from(i) }))
            .collect()
    }

    /// `None` when the rows are out of order or name an entity that is gone.
    pub(crate) fn from_rec(rows: Rows, exists: impl Fn(EntityId) -> bool) -> Option<StartEffects> {
        let mut out = Vec::with_capacity(rows.len());
        for (at, id, act) in rows {
            let id = EntityId(id);
            let act = match act {
                0 => Act::Expire,
                n if n <= 256 => Act::Replace((n - 1) as u8),
                _ => return None,
            };
            if !exists(id) {
                return None;
            }
            out.push(Row { at, id, act });
        }
        out.windows(2).all(|w| w[0] < w[1]).then_some(StartEffects { rows: out })
    }

    /// Drop the rows of a removed entity.
    pub(crate) fn remove(&mut self, id: EntityId) {
        self.rows.retain(|r| r.id != id);
    }

    fn take_due(&mut self, tick: Tick) -> Vec<Row> {
        let n = self.rows.partition_point(|r| r.at <= tick);
        self.rows.drain(..n).collect()
    }
}

impl Sim {
    fn owner_kind(&self, owner: PlayerId) -> Owner {
        if owner == NEUTRAL {
            Owner::Derelict
        } else if self.ai_seats.binary_search(&owner).is_ok() {
            Owner::Ai
        } else {
            Owner::Player
        }
    }

    /// Schedule the lifecycle of a freshly spawned entity.
    pub(crate) fn register_lifecycle(&mut self, id: EntityId) {
        let Some(db) = self.content.clone() else { return };
        let Some(u) = self.units.get(id) else { return };
        let Some(def) = db.unit(u.def).filter(|d| d.hidden) else { return };
        let kind = self.owner_kind(u.owner);
        if let Some(life) = def.life {
            self.start_effects.add(Row { at: self.tick + life, id, act: Act::Expire });
        }
        for (i, hp) in def.hardpoints.iter().enumerate().take(256) {
            if let Some(r) = db.weapon(hp.weapon.key).and_then(|w| w.replace.as_ref()).filter(|r| r.fires_for(kind)) {
                self.start_effects.add(Row { at: self.tick + r.delay, id, act: Act::Replace(i as u8) });
            }
        }
    }

    /// Run what is due this tick.
    pub(crate) fn step_start_effects(&mut self) {
        if self.start_effects.is_empty() {
            return;
        }
        let Some(db) = self.content.clone() else { return };
        for row in self.start_effects.take_due(self.tick) {
            match row.act {
                Act::Expire => {
                    self.despawn(row.id, None);
                }
                Act::Replace(hp) => self.fire_replacement(&db, row.id, hp),
            }
        }
    }

    fn fire_replacement(&mut self, db: &ContentDb, id: EntityId, hardpoint: u8) {
        let Some(source) = self.units.get(id) else { return };
        let Some(def) = db.unit(source.def) else { return };
        let Some(weapon) = def.hardpoints.get(hardpoint as usize).and_then(|hp| db.weapon(hp.weapon.key)) else { return };
        let Some(r) = weapon.replace.as_ref() else { return };
        let (owner, pos, heading, source_odf) = (source.owner, source.pos, source.heading, def.odf.clone());
        if let Some(need) = r.required_officers {
            let reached = self.economy.officer_limit(owner).is_some_and(|limit| limit >= Fixed::from_int(need.clamp(0, 1_000_000_000) as i32));
            if !reached {
                return;
            }
        }
        let picks = self.pick_replacements(r, &source_odf);
        let mut spawned = false;
        for (odf, offset) in picks {
            let Some(key) = db.key(&odf).filter(|&k| db.unit(k).is_some()) else { continue };
            let at = Vec3 { x: pos.x + offset[0], y: pos.y + offset[1], z: pos.z + offset[2] };
            spawned |= self.spawn_def(key, owner, at, heading).is_some();
        }
        if spawned {
            self.despawn(id, None);
        }
    }

    /// The classes a replacement weapon produces now, with their offsets.
    fn pick_replacements(&mut self, r: &ReplaceDef, source_odf: &str) -> Vec<(String, [Fixed; 3])> {
        let candidates = r.entries.iter().filter(|e| e.condition.as_deref().is_none_or(|c| c.eq_ignore_ascii_case(source_odf)));
        let chosen: Vec<_> = match r.mode {
            0 => candidates.collect(),
            2 => candidates.take(1).collect(),
            _ => Vec::new(),
        };
        let mut out = Vec::new();
        for entry in chosen {
            let total: u64 = entry.classes.iter().map(|c| u64::from(c.weight)).sum();
            let class = if total > 0 {
                let mut roll = self.rng.next_u64() % total;
                entry.classes.iter().filter(|c| c.weight > 0).find(|c| {
                    if roll < u64::from(c.weight) {
                        true
                    } else {
                        roll -= u64::from(c.weight);
                        false
                    }
                })
            } else {
                entry.classes.first()
            };
            out.extend(class.map(|c| (c.odf.clone(), c.offset)));
        }
        out
    }
}

/// What of the lifecycle of the start entry `key` this sim does not run, one
/// note per weapon or class (naming its ODF key); empty when everything it
/// does is supported. Follows the classes its replacement weapons produce.
pub fn unsupported(db: &ContentDb, key: sta_core::ContentKey) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = vec![key];
    let mut queue = vec![key];
    while let Some(k) = queue.pop() {
        let Some(def) = db.unit(k) else { continue };
        for hp in &def.hardpoints {
            let Some(w) = db.weapon(hp.weapon.key) else { continue };
            match &w.replace {
                Some(r) => {
                    if !matches!(r.mode, 0 | 2) {
                        out.push(format!("{}: replacementMode {} is not simulated", w.odf, r.mode));
                    }
                    for class in r.entries.iter().flat_map(|e| &e.classes) {
                        match db.key(&class.odf).filter(|&c| db.unit(c).is_some()) {
                            Some(c) if !seen.contains(&c) => {
                                seen.push(c);
                                queue.push(c);
                            }
                            Some(_) => {}
                            None => out.push(format!("{}: replacement class {} is not a unit; not spawned", w.odf, class.odf)),
                        }
                    }
                }
                None if def.class.0 == "u_ball_obj" || def.hidden => {
                    out.push(format!("{}: {} weapon (class {}) is not simulated", def.odf, w.odf, w.class.0));
                }
                None => {}
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// [`unsupported`] as startup warnings for a race's entry. Only `hidden = 1`
/// entries (the helpers and effects whose lifecycle this is) are reported:
/// an ordinary ship's own weapons are not a start concern.
pub(crate) fn warnings(db: &ContentDb, race_file: &str, race: &str, odf: &str, key: sta_core::ContentKey) -> Vec<Warning> {
    if !db.unit(key).is_some_and(|d| d.hidden) {
        return Vec::new();
    }
    unsupported(db, key)
        .into_iter()
        .map(|note| Warning::new(race_file, format!("race {race}: standardUnits entry for {odf}.odf: {note}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sta_content::lifecycle::{ReplaceClass, ReplaceEntry};
    use sta_content::{ContentDb, DefRef, Hardpoint, UnitDef, WeaponDef, WeaponKind};
    use sta_core::TeamId;

    use super::*;
    use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use crate::WorldSource;

    fn pod(name: &str, life: Option<Tick>, weapons: &[&str]) -> UnitDef {
        let mut d = UnitDef::new(name, "pod");
        d.hidden = true;
        d.life = life;
        d.hull = Fixed::from_int(100);
        for (i, w) in weapons.iter().enumerate() {
            d.hardpoints.push(Hardpoint { weapon: DefRef::new(*w), node: format!("hp{i}"), arc: None });
        }
        d
    }

    fn class(odf: &str, weight: u32, x: i32) -> ReplaceClass {
        ReplaceClass { odf: odf.into(), weight, offset: [Fixed::from_int(x), Fixed::ZERO, Fixed::ZERO] }
    }

    fn replace(name: &str, instant: [bool; 3], delay: Tick, mode: i64, required: Option<i64>, entries: Vec<ReplaceEntry>) -> WeaponDef {
        let mut w = WeaponDef::new(name, "replaceweapon", WeaponKind::Beam);
        w.replace = Some(ReplaceDef { instant, delay, mode, required_officers: required, entries });
        w
    }

    fn entry(condition: Option<&str>, classes: Vec<ReplaceClass>) -> ReplaceEntry {
        ReplaceEntry { condition: condition.map(str::to_owned), classes }
    }

    fn db() -> ContentDb {
        let mut plain = UnitDef::new("plain", "craft");
        plain.hull = Fixed::from_int(10);
        plain.life = Some(3); // not hidden: no lifecycle
        ContentDb::builder()
            .unit(pod("src", None, &["to_ai", "to_player", "late"]))
            .unit(pod("fresh_ai", Some(30), &[]))
            .unit(pod("fresh_a", None, &[]))
            .unit(pod("fresh_b", None, &[]))
            .unit(pod("fleeting", Some(15), &[]))
            .unit(pod("gated", None, &["need_officers"]))
            .unit(pod("unitcap", None, &[]))
            .unit(plain)
            .weapon(replace("to_ai", [false, true, false], 0, 2, None, vec![entry(Some("src"), vec![class("fresh_ai", 0, 0)])]))
            .weapon(replace(
                "to_player", [true, false, false], 6, 0, None,
                vec![entry(None, vec![class("fresh_a", 0, -50)]), entry(Some("other"), vec![class("fleeting", 0, 0)]), entry(Some("SRC"), vec![class("fresh_b", 0, 100)])],
            ))
            .weapon(replace("late", [true, true, true], 600, 0, None, vec![entry(None, vec![class("fleeting", 0, 0)])]))
            .weapon(replace("need_officers", [true, true, true], 30, 2, Some(299_999), vec![entry(Some("gated"), vec![class("unitcap", 0, 0)])]))
            .build()
            .unwrap()
    }

    fn game(players: &[(u8, Control)]) -> (Sim, Arc<ContentDb>) {
        let content = Arc::new(db());
        let setup = GameSetup {
            seed: 5,
            map: Arc::new(sta_content::fake::two_bases()),
            content: content.clone(),
            players: players.iter().map(|(i, c)| PlayerSetup { id: PlayerId(*i), team: TeamId(*i), race: "fx".into(), control: c.clone() }).collect(),
            options: GameOptions::default(),
        };
        (Sim::from_setup(&setup), content)
    }

    fn put(sim: &mut Sim, db: &ContentDb, odf: &str, owner: PlayerId) -> EntityId {
        sim.spawn_def(db.key(odf).unwrap(), owner, Vec3 { x: Fixed::from_int(1000), y: Fixed::ZERO, z: Fixed::from_int(2000) }, (Fixed::ZERO, Fixed::ONE)).unwrap()
    }

    fn names(sim: &Sim, db: &ContentDb) -> Vec<String> {
        let mut v: Vec<_> = sim.units.iter().map(|u| db.unit(u.def).unwrap().odf.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn hidden_objects_expire_after_their_lifetime_and_others_do_not() {
        let (mut sim, db) = game(&[(0, Control::Human)]);
        put(&mut sim, &db, "fleeting", PlayerId(0));
        put(&mut sim, &db, "plain", PlayerId(0));
        for _ in 0..15 {
            sim.step(&[]);
        }
        assert_eq!(names(&sim, &db), ["fleeting", "plain"]);
        sim.step(&[]);
        assert_eq!(names(&sim, &db), ["plain"]);
    }

    #[test]
    fn an_ai_seat_replaces_at_once_and_a_player_seat_at_its_delay_with_every_matching_entry() {
        let (mut sim, db) = game(&[(0, Control::Human), (1, Control::Ai("x".into()))]);
        put(&mut sim, &db, "src", PlayerId(1));
        sim.step(&[]);
        assert_eq!(names(&sim, &db), ["fresh_ai"]);
        assert_eq!(sim.units.iter().next().unwrap().owner, PlayerId(1));

        let src = put(&mut sim, &db, "src", PlayerId(0));
        let pos = sim.unit(src).unwrap().pos;
        for _ in 0..7 {
            sim.step(&[]);
        }
        // Entry 1's condition names another source; entries 0 and 2 apply (mode 0), case-blind.
        assert_eq!(names(&sim, &db), ["fresh_a", "fresh_ai", "fresh_b"].map(String::from));
        let at = |n: &str| sim.units.iter().find(|u| db.unit(u.def).unwrap().odf == n).unwrap().pos;
        assert_eq!(at("fresh_a").x, pos.x - Fixed::from_int(50));
        assert_eq!(at("fresh_b").x, pos.x + Fixed::from_int(100));
        assert_eq!(at("fresh_b").z, pos.z);
    }

    #[test]
    fn nothing_fires_for_an_owner_kind_the_weapon_does_not_name() {
        // `src` has an AI-only weapon first and a player one second: a player seat
        // sees only the second, a derelict seat only `late`.
        let (mut sim, db) = game(&[(0, Control::Human)]);
        let id = put(&mut sim, &db, "src", NEUTRAL);
        for _ in 0..30 {
            sim.step(&[]);
        }
        assert!(sim.unit(id).is_some(), "no weapon of `src` is for a derelict before `late`");
    }

    #[test]
    fn required_officers_need_the_owners_maximum() {
        let (mut sim, db) = game(&[(0, Control::Human)]);
        sim.economy.set_personnel(PlayerId(0), None, Some(Fixed::from_int(99_999)));
        let low = put(&mut sim, &db, "gated", PlayerId(0));
        for _ in 0..40 {
            sim.step(&[]);
        }
        assert!(sim.unit(low).is_some());
        sim.economy.set_personnel(PlayerId(0), None, Some(Fixed::from_int(299_999)));
        let high = put(&mut sim, &db, "gated", PlayerId(0));
        for _ in 0..31 {
            sim.step(&[]);
        }
        assert!(sim.unit(high).is_none());
        assert_eq!(names(&sim, &db).iter().filter(|n| *n == "unitcap").count(), 1);
    }

    #[test]
    fn weights_draw_from_the_sim_generator_deterministically() {
        let pick = |seed: u64| {
            let mut sim = Sim::new(seed);
            let r = ReplaceDef { instant: [true; 3], delay: 0, mode: 2, required_officers: None, entries: vec![entry(None, vec![class("a", 35, 0), class("b", 35, 0), class("c", 25, 0)])] };
            (0..64).map(|_| sim.pick_replacements(&r, "x")[0].0.clone()).collect::<Vec<_>>()
        };
        assert_eq!(pick(9), pick(9));
        let drawn = pick(9);
        assert!(["a", "b", "c"].iter().all(|n| drawn.iter().any(|d| d == n)), "{drawn:?}");
    }

    #[test]
    fn pending_rows_survive_snapshot_and_replay_state() {
        let (mut sim, db) = game(&[(0, Control::Human), (1, Control::Ai("x".into()))]);
        put(&mut sim, &db, "src", PlayerId(0));
        put(&mut sim, &db, "fleeting", PlayerId(1));
        let empty = Sim::new(1).digest();
        assert_ne!(sim.digest(), empty);
        for _ in 0..4 {
            sim.step(&[]);
        }
        let mut restored = Sim::from_snapshot(&sim.snapshot(), Some(db.clone())).unwrap();
        assert_eq!(restored.digest(), sim.digest());
        for _ in 0..20 {
            sim.step(&[]);
            restored.step(&[]);
            assert_eq!(restored.digest(), sim.digest());
        }
        assert_eq!(names(&restored, &db), names(&sim, &db));
    }

    #[test]
    fn unsupported_lists_what_is_not_run() {
        let mut ball = UnitDef::new("ball", "u_ball_obj");
        ball.hidden = true;
        ball.hardpoints.push(Hardpoint { weapon: DefRef::new("reveal"), node: "root".into(), arc: None });
        let mut head = pod("head", None, &["go", "bad_mode"]);
        head.hardpoints.push(Hardpoint { weapon: DefRef::new("go"), node: "root".into(), arc: None });
        let db = ContentDb::builder()
            .unit(ball)
            .unit(head)
            .weapon(WeaponDef::new("reveal", "cannonimp", WeaponKind::Beam))
            .weapon(replace("go", [true; 3], 0, 0, None, vec![entry(None, vec![class("ball", 0, 0), class("ghost", 0, 0)])]))
            .weapon(replace("bad_mode", [true; 3], 0, 1, None, vec![]))
            .build()
            .unwrap();
        let notes = unsupported(&db, db.key("head").unwrap());
        assert_eq!(
            notes,
            [
                "bad_mode: replacementMode 1 is not simulated",
                "ball: reveal weapon (class cannonimp) is not simulated",
                "go: replacement class ghost is not a unit; not spawned",
            ]
        );
        assert!(unsupported(&db, db.key("ball").unwrap()).len() == 1);
    }
}
