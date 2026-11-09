//! Tech availability (SIM-09): which defs a player may build, given what
//! they own, from the content db's tech tree (CON-06).
//!
//! - [`Owned`] is the [`TechState`] of one player, read off the entities.
//!   It counts **finished** entities of the player (a station still being
//!   welded does not count; a queue item does not exist yet) by def name,
//!   which is the ODF stem the tree uses. The race is the builder's race.
//!   An ally is another seat on the player's team that owns an entity of the
//!   asked race. Everything is read by ascending entity id and a sorted
//!   player list, so it is deterministic and holds no state of its own:
//!   nothing here goes in the snapshot or the digest.
//! - [`build_allowed`] gates `Order::Build` (called from `Sim::apply`). A
//!   refusal is the ordinary rejection, [`SimEvent::Rejected`](crate::SimEvent),
//!   like every other refused build.
//! - A db without a tech tree (`ContentDb::tech_tree()` is `None`, as for
//!   `fake::db()`) allows everything. When there is a tree, a def it does not
//!   list ([`Availability::NotListed`]) is blocked unless
//!   [`sta_content::TechTree::availability_for_builder`] lets it through: a
//!   def in the db (never an `edit_` page) whose build slot carries an
//!   availability or provided line, or whose builder flags no slot at all,
//!   is open to everyone; a bare slot of a builder that flags others is an
//!   AI/engine tail slot, open to AI seats (`Control::Ai` in the setup, kept
//!   as `Sim::ai_seats`) and blocked for humans. The HUD's build page asks
//!   the same function. The engine's own handling of unlisted ODFs is
//!   undocumented; the rule is inferred from install data.
//! - Checked when the order is accepted, not again when a queued item
//!   starts or a rig deploys.

use sta_content::{Availability, ContentDb, TechState};
use sta_core::{ContentKey, EntityId, Fixed, PlayerId, TeamId};

use crate::world::{Entities, Entity};

/// One player's holdings as the tech tree asks for them.
pub struct Owned<'a> {
    db: &'a ContentDb,
    units: &'a Entities,
    player: PlayerId,
    race: String,
    /// Other seats on the player's team, ascending.
    allies: Vec<PlayerId>,
}

impl<'a> Owned<'a> {
    pub fn new(
        db: &'a ContentDb,
        units: &'a Entities,
        seats: &[(PlayerId, TeamId)],
        player: PlayerId,
        race: &str,
    ) -> Owned<'a> {
        let team = seats.iter().find(|s| s.0 == player).map(|s| s.1);
        let allies = seats
            .iter()
            .filter(|s| s.0 != player && Some(s.1) == team)
            .map(|s| s.0)
            .collect();
        Owned { db, units, player, race: race.to_ascii_lowercase(), allies }
    }
}

fn finished(e: &Entity) -> bool {
    e.progress.is_none_or(|p| p >= Fixed::ONE)
}

impl TechState for Owned<'_> {
    fn count(&self, odf: &str) -> u32 {
        let Some(key) = self.db.key(odf) else { return 0 };
        self.units
            .iter()
            .filter(|e| e.owner == self.player && e.def == key && finished(e))
            .count() as u32
    }

    fn race(&self) -> &str {
        &self.race
    }

    fn has_ally(&self, race: &str) -> bool {
        self.units.iter().any(|e| {
            finished(e)
                && self.allies.contains(&e.owner)
                && self.db.unit(e.def).is_some_and(|d| d.race.eq_ignore_ascii_case(race))
        })
    }
}

/// Whether `builder`'s owner may build `what` now. False for an unknown
/// builder (the order is refused anyway).
pub(crate) fn build_allowed(
    db: &ContentDb,
    units: &Entities,
    seats: &[(PlayerId, TeamId)],
    ai_seats: &[PlayerId],
    builder: EntityId,
    what: ContentKey,
) -> bool {
    let Some(tree) = db.tech_tree() else { return true };
    let Some(b) = units.get(builder) else { return false };
    let race = db.unit(b.def).map_or("", |d| d.race.as_str());
    let Some(name) = db.name(what) else { return false };
    let state = Owned::new(db, units, seats, b.owner, race);
    matches!(tree.availability_for_builder(db, name, db.name(b.def).unwrap_or(""), &state, ai_seats.contains(&b.owner)), Availability::Available)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Command, CommandKind, Order};
    use crate::setup::{Control, GameOptions, GameSetup, PlayerSetup};
    use crate::{Sim, SimEvent, WorldSource};
    use sta_content::{TechTree, UnitDef};
    use sta_core::Vec3;
    use std::sync::Arc;

    const P0: PlayerId = PlayerId(0);
    const P1: PlayerId = PlayerId(1);

    /// `fake::db()` rebuilt, with a tech tree when given.
    fn game(tree: Option<&str>, teams: [u8; 2]) -> Sim {
        game_with(tree, teams, |_| {})
    }

    /// An ODF of `text`, as a loaded unit's `raw` is.
    fn raw(name: &str, text: &str) -> Arc<sta_odf::Odf> {
        Arc::new(sta_odf::resolve(name, &mut |_| Some(text.to_owned())).unwrap())
    }

    /// [`game`] with every unit passed through `patch` first, and an
    /// `edit_page` unit (a map-editor page) added.
    fn game_with(tree: Option<&str>, teams: [u8; 2], patch: impl Fn(&mut UnitDef)) -> Sim {
        let fake = sta_content::fake::db();
        let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for w in fake.weapons() {
            b = b.weapon(w.clone());
        }
        for u in fake.units() {
            let mut u = u.clone();
            patch(&mut u);
            b = b.unit(u);
        }
        if tree.is_some() {
            b = b.unit(UnitDef::new("edit_page", "craft"));
        }
        if let Some(t) = tree {
            let (t, w) = TechTree::parse("t.tt", t);
            assert!(w.is_empty(), "{w:?}");
            b = b.tech_tree(t);
        }
        let seat = |id: u8, team: u8| PlayerSetup {
            id: PlayerId(id),
            team: TeamId(team),
            race: "fx".into(),
            control: if id == 0 { Control::Human } else { Control::Ai("test".into()) },
        };
        Sim::from_setup(&GameSetup {
            seed: 3,
            map: Arc::new(sta_content::MapDef::empty()),
            content: Arc::new(b.build().unwrap()),
            players: vec![seat(0, teams[0]), seat(1, teams[1])],
            options: GameOptions::default(),
        })
    }

    fn at() -> Vec3 {
        Vec3 { x: Fixed::ZERO, y: Fixed::ZERO, z: Fixed::ZERO }
    }

    fn spawn(sim: &mut Sim, odf: &str, owner: PlayerId) -> EntityId {
        let k = sim.content().unwrap().key(odf).unwrap();
        sim.spawn_def(k, owner, at(), (Fixed::ZERO, Fixed::ONE)).unwrap()
    }

    /// Whether the build order is accepted.
    fn build(sim: &mut Sim, who: PlayerId, yard: EntityId, odf: &str) -> bool {
        let what = sim.content().unwrap().key(odf).unwrap();
        let c = Command { player: who, seq: 1, kind: CommandKind::Order(Order::Build { builder: yard, what, at: None }) };
        !sim.step(&[c]).events.iter().any(|e| matches!(e, SimEvent::Rejected { .. }))
    }

    const TREE: &str = "fx_cruiser 0\nfx_frigate 1 fx_station\nfx_yard 0\nfx_station 0\n";

    #[test]
    fn rejected_until_the_prerequisite_exists() {
        let mut sim = game(Some(TREE), [0, 1]);
        let yard = spawn(&mut sim, "fx_yard", P0);
        assert!(build(&mut sim, P0, yard, "fx_cruiser"));
        assert!(!build(&mut sim, P0, yard, "fx_frigate"));
        // Another player's station does not unlock it.
        spawn(&mut sim, "fx_station", P1);
        assert!(!build(&mut sim, P0, yard, "fx_frigate"));
        // One still welding does not either.
        let welding = spawn(&mut sim, "fx_station", P0);
        sim.units.get_mut(welding).unwrap().progress = Some(Fixed::from_f64(0.5));
        assert!(!build(&mut sim, P0, yard, "fx_frigate"));
        sim.units.get_mut(welding).unwrap().progress = None;
        assert!(build(&mut sim, P0, yard, "fx_frigate"));
    }

    #[test]
    fn a_def_the_tree_does_not_list_is_blocked() {
        // The tree lists the frigate only. The cruiser is a bare slot of a
        // yard that flags another one, so a human cannot build it; the
        // flagged slot (`edit_page` is the yard's slot 2, see `yard_game`)
        // does not change that.
        let mut sim = yard_game("fx_frigate 0\nfx_yard 0\n", false, false);
        let yard = spawn(&mut sim, "fx_yard", P0);
        assert!(build(&mut sim, P0, yard, "fx_frigate"));
        assert!(!build(&mut sim, P0, yard, "fx_cruiser"));
    }

    /// The yard's slot 1 holds the cruiser, flagged (an availability line)
    /// when `flagged`; slot 3 is flagged too unless `menu`, which leaves the
    /// yard with no flag at all.
    fn yard_game(tree: &str, flagged: bool, menu: bool) -> Sim {
        game_with(Some(tree), [0, 1], |u| {
            if u.odf == "fx_yard" {
                let slot1 = if flagged { "builditem1availability = 0\n" } else { "" };
                let other = if menu { "" } else { "builditem3provided = 1\n" };
                u.raw = raw("fx_yard", &format!("builditem1 = \"fx_cruiser\"\n{slot1}builditem2 = \"edit_page\"\n{other}"));
                u.build.list.push(sta_content::DefRef::new("edit_page"));
            }
        })
    }

    #[test]
    fn a_flagged_slot_is_allowed_to_humans() {
        let mut sim = yard_game("fx_yard 0\n", true, false);
        let yard = spawn(&mut sim, "fx_yard", P0);
        assert!(build(&mut sim, P0, yard, "fx_cruiser"));
    }

    #[test]
    fn a_builder_with_no_flags_offers_unlisted_slots_to_humans() {
        let mut sim = yard_game("fx_yard 0\n", false, true);
        let yard = spawn(&mut sim, "fx_yard", P0);
        assert!(build(&mut sim, P0, yard, "fx_cruiser"));
    }

    #[test]
    fn a_tail_slot_is_blocked_for_humans_and_allowed_for_ai() {
        // Slot 1 is bare while slot 3 is flagged. P0 is Human, P1 is Ai.
        let mut sim = yard_game("fx_yard 0\n", false, false);
        let mine = spawn(&mut sim, "fx_yard", P0);
        let theirs = spawn(&mut sim, "fx_yard", P1);
        assert!(!build(&mut sim, P0, mine, "fx_cruiser"));
        assert!(build(&mut sim, P1, theirs, "fx_cruiser"));
    }

    #[test]
    fn editor_pages_are_blocked_even_for_ai() {
        let mut sim = yard_game("fx_yard 0\n", false, true);
        let theirs = spawn(&mut sim, "fx_yard", P1);
        assert!(!build(&mut sim, P1, theirs, "edit_page"));
    }

    #[test]
    fn a_withheld_def_stays_blocked_even_for_ai() {
        let mut sim = yard_game("fx_yard 0\nfx_cruiser -1\n", true, true);
        let mine = spawn(&mut sim, "fx_yard", P0);
        let theirs = spawn(&mut sim, "fx_yard", P1);
        assert!(!build(&mut sim, P0, mine, "fx_cruiser"));
        assert!(!build(&mut sim, P1, theirs, "fx_cruiser"));
    }

    #[test]
    fn no_tree_allows_everything() {
        let mut sim = game(None, [0, 1]);
        let yard = spawn(&mut sim, "fx_yard", P0);
        assert!(build(&mut sim, P0, yard, "fx_cruiser"));
        assert!(build(&mut sim, P0, yard, "fx_frigate"));
    }

    #[test]
    fn allies_are_teammates_owning_the_race() {
        let mut sim = game(None, [0, 0]);
        spawn(&mut sim, "fx_cruiser", P1);
        let db = sim.content().unwrap().clone();
        let race = db.unit(db.key("fx_cruiser").unwrap()).unwrap().race.clone();
        let team = [(P0, TeamId(0)), (P1, TeamId(0))];
        let o = Owned::new(&db, &sim.units, &team, P0, "FX");
        assert_eq!(o.race(), "fx");
        assert!(o.has_ally(&race));
        let apart = [(P0, TeamId(0)), (P1, TeamId(1))];
        assert!(!Owned::new(&db, &sim.units, &apart, P0, "fx").has_ally(&race));
    }

    #[test]
    fn tree_changes_the_content_hash_only_when_present() {
        let a = game(None, [0, 1]);
        let b = game(Some(TREE), [0, 1]);
        let c = game(Some(TREE), [0, 1]);
        assert_ne!(a.content().unwrap().hash(), b.content().unwrap().hash());
        assert_eq!(b.content().unwrap().hash(), c.content().unwrap().hash());
    }
}
