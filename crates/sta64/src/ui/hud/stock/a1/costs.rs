//! What a build or research item costs, in words (UI-42).
//!
//! The Armada I build and research buttons (research pods are build items of a research
//! station) carry a `needs:` line in their tooltip: the item's cost for each resource slot and the
//! officers it takes, read from the unit definition's `build` data (the same cost the sim charges),
//! with the words from the mounted string table: `GUI_CP_REQUIRE`, `GUI_CP_DILITHIUM_RES`,
//! `GUI_CP_OFFICERS_RES` and the race variants (`GUI_CP_BORGOFF_RES`, `GUI_CP_ROMOFF_RES`).
//! Evidence: the keys are read from data (`Dynamic_Localized_Strings.h`); that the original lists
//! them behind "needs:" for build and research buttons is read from the executable's string
//! table, static. Its output order, whether it shows build time, and where it draws it are NOT
//! established: this is the interim rule (STA64's own tooltip, content only; the line goes in the
//! short and the verbose tooltip because the short one shows first). Ordinary unit crew is not a
//! build cost. A1 officer-quarter upgrades separately consume the authored crew amount.
//!
//! The amounts are the definition's cost, unscaled: a seat race's cost scaling in the sim (the latinum
//! factor of a race the data frees of latinum) is not reflected (TODO).
//!
//! The missing resource is named from `sta_sim::production::afford::build_shortfall`, the sim's
//! own resource/officer payment test over the client's view. The separate A1 upgrade crew check
//! uses the same player's crew stock.

use sta_content::{ContentDb, ResourceModel};
use sta_sim::{ContentKey, Fixed};
use sta_sim::production::afford::{Shortfall, build_shortfall};

use crate::ui::{
    hud::model::HudFeed,
    i18n::{Strings, tr, trf},
    profile::UiProfile,
};

/// The cost of an item, whole units.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Needs {
    /// `(resource slot, amount)` for each positive cost.
    pub slots: Vec<(usize, i64)>,
    pub officers: i64,
    /// Crew consumed by an A1 officer-quarter upgrade only.
    pub crew: i64,
    /// The unit's race name, for the officers word.
    pub race: String,
}

/// The amount shown for a cost the sim charges as it stands: rounded up to the next whole unit, so
/// a fractional cost (0.5, 1.9) never reads as less than what is charged. A choice of display
/// (the original's rounding is not established), covered by a test.
fn whole(v: f32) -> i64 {
    v.ceil() as i64
}

/// An item's cost from its definition; `None` for an unknown key.
pub(in crate::ui::hud) fn needs(db: &ContentDb, key: ContentKey) -> Option<Needs> {
    let def = db.unit(key)?;
    Some(Needs {
        slots: def
            .build
            .cost
            .iter()
            .enumerate()
            .filter(|(_, c)| c.to_f32() > 0.0)
            .map(|(i, c)| (i, whole(c.to_f32())))
            .collect(),
        officers: whole(def.build.officers.to_f32().max(0.0)),
        crew: if db.a1_officers() && def.a1_officer_upgrade_gain.is_some() { whole(def.crew.to_f32().max(0.0)) } else { 0 },
        race: def.race.clone(),
    })
}

/// A game string if the table has it, else `fallback`.
fn game_word(strings: Option<&Strings>, key: &str, fallback: String) -> String {
    match strings.map(|s| s.tooltip_text(key)) {
        Some(text) if text != key && !text.is_empty() => text.to_owned(),
        _ => fallback,
    }
}

/// The word for a resource slot: the game's `GUI_CP_DILITHIUM_RES` for dilithium (slot 0), else the
/// resource model's own name, lower case.
fn slot_word(strings: Option<&Strings>, resources: &ResourceModel, slot: usize) -> String {
    let own = resources.name(slot).to_lowercase();
    if slot == sta_content::resources::DILITHIUM { game_word(strings, "GUI_CP_DILITHIUM_RES", own) } else { own }
}

/// The word for officers: the race's own key (`GUI_CP_<RACE>OFF_RES`, three or four letters of the
/// race name) when the table has one, else `GUI_CP_OFFICERS_RES`.
fn officers_word(strings: Option<&Strings>, race: &str) -> String {
    let default = game_word(strings, "GUI_CP_OFFICERS_RES", tr("officers"));
    let race = race.trim().to_ascii_uppercase();
    [4, 3]
        .into_iter()
        .filter_map(|n| race.get(..n))
        .map(|p| format!("GUI_CP_{p}OFF_RES"))
        .find_map(|key| strings.map(|s| s.tooltip_text(&key)).filter(|t| *t != key && !t.is_empty()).map(str::to_owned))
        .unwrap_or(default)
}

fn short_word(strings: Option<&Strings>, resources: &ResourceModel, needs: &Needs, shortfall: Shortfall) -> String {
    match shortfall {
        Shortfall::Resource(slot) => slot_word(strings, resources, slot),
        Shortfall::Officers => officers_word(strings, &needs.race),
    }
}

/// `needs: 120 dilithium, 5 officers`, with `(short of: dilithium)` after it when the player cannot
/// pay now. Empty when the item costs nothing.
pub(in crate::ui::hud) fn line(
    strings: Option<&Strings>,
    resources: &ResourceModel,
    needs: &Needs,
    shortfall: Option<Shortfall>,
) -> String {
    let mut parts: Vec<String> = needs.slots.iter().map(|&(slot, n)| format!("{n} {}", slot_word(strings, resources, slot))).collect();
    if needs.officers > 0 {
        parts.push(format!("{} {}", needs.officers, officers_word(strings, &needs.race)));
    }
    if needs.crew > 0 {
        parts.push(format!("{} {}", needs.crew, tr("crew")));
    }
    if parts.is_empty() {
        return String::new();
    }
    let mut out = format!("{} {}", game_word(strings, "GUI_CP_REQUIRE", tr("needs:")), parts.join(", "));
    if let Some(short) = shortfall {
        out.push_str(&format!(" ({})", trf("short of: {0}", &[&short_word(strings, resources, needs, short)])));
    }
    out
}

/// What the player lacks for `key` now, from the sim's own payment test (`None`: nothing, or no
/// player view yet).
pub(in crate::ui::hud) fn shortfall(feed: &HudFeed, key: ContentKey) -> Option<Shortfall> {
    let db = feed.content.as_deref()?;
    build_shortfall(db, feed.view.player(feed.player)?, key)
}

/// The A1 quarter's separate player-crew charge. An absent pool has zero available crew.
pub(in crate::ui::hud) fn crew_shortfall(feed: &HudFeed, key: ContentKey) -> bool {
    let Some(db) = feed.content.as_deref() else { return false };
    let Some(def) = db.unit(key) else { return false };
    if !db.a1_officers() || def.a1_officer_upgrade_gain.is_none() { return false; }
    let Some(player) = feed.view.player(feed.player) else { return false };
    player.crew.unwrap_or(Fixed::ZERO) < def.crew.max(Fixed::ZERO)
}

/// The refusal notice for a build the player cannot pay for, Armada I only (the other profiles keep
/// their wording): `Cannot build: not enough dilithium`.
pub(in crate::ui::hud) fn refusal(feed: &HudFeed, profile: Option<&UiProfile>, strings: Option<&Strings>, key: ContentKey) -> Option<String> {
    if !super::is_a1(profile) {
        return None;
    }
    let db = feed.content.as_deref()?;
    let needs = needs(db, key)?;
    if shortfall(feed, key).is_none() && crew_shortfall(feed, key) {
        return Some(trf("Cannot build: not enough {0}", &[&tr("crew")]));
    }
    let short = shortfall(feed, key)?;
    let word = short_word(strings, &crate::ui::hud::model::resource_model(feed), &needs, short);
    Some(trf("Cannot build: not enough {0}", &[&word]))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sta_content::ButtonSlots;
    use sta_sim::fake::ScriptedSource;
    use sta_sim::view::PlayerView;
    use sta_sim::{EntityId, Fixed, PlayerId, TeamId, WorldSource};

    use super::*;
    use crate::ui::hud::{
        stock::a1::control,
        model::{Act, Page},
    };

    /// The fake content with the station's cost set to arbitrary authored numbers.
    fn db(dilithium: i32, officers: i32, race: &str) -> Arc<ContentDb> {
        let fake = sta_content::fake::db();
        let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for w in fake.weapons() {
            b = b.weapon(w.clone());
        }
        for u in fake.units() {
            let mut u = u.clone();
            if u.odf == "fx_cruiser" {
                u.build.list = vec![sta_content::DefRef::new("fx_station")];
            }
            if u.odf == "fx_station" {
                u.build.cost = vec![Fixed::from_int(dilithium), Fixed::ZERO, Fixed::ZERO, Fixed::ZERO];
                u.build.officers = Fixed::from_int(officers);
                u.race = race.to_owned();
            }
            b = b.unit(u);
        }
        Arc::new(b.build().unwrap())
    }

    fn feed(db: &Arc<ContentDb>, stock: i32, officers: [i32; 2]) -> HudFeed {
        let mut source = ScriptedSource::from_json5(
            "{ units: [{ id: 1, def: 'fx_cruiser', owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] }] }",
            db,
        )
        .unwrap();
        let mut view = (*source.step(&[]).view).clone();
        view.players = vec![PlayerView {
            crew: None,
            id: PlayerId(1),
            team: TeamId(1),
            resources: vec![Fixed::from_int(stock), Fixed::ZERO, Fixed::ZERO, Fixed::ZERO],
            officers: officers.map(Fixed::from_int),
            eliminated: false,
            relations: Vec::new(),
        }];
        HudFeed { view: Arc::new(view), player: PlayerId(1), selected: vec![EntityId(1)], content: Some(db.clone()), ..Default::default() }
    }

    fn station(db: &ContentDb) -> ContentKey {
        db.key("fx_station").unwrap()
    }

    #[test]
    fn the_line_lists_every_cost_the_sim_charges() {
        let db = db(120, 5, "x");
        let n = needs(&db, station(&db)).unwrap();
        assert_eq!((n.slots.clone(), n.officers), (vec![(0, 120)], 5));
        let line = line(None, &ResourceModel::default(), &n, None);
        assert_eq!(line, "needs: 120 dilithium, 5 officers");
        let free = db_free();
        assert_eq!(super::line(None, &ResourceModel::default(), &free, None), "", "a free item has no line");
    }

    fn db_free() -> Needs {
        Needs::default()
    }

    #[test]
    fn the_words_come_from_the_string_table_with_race_variants() {
        let table = "\"GUI_CP_REQUIRE\", \"requires\", \"GUI_CP_DILITHIUM_RES\", \"deut\", \"GUI_CP_OFFICERS_RES\", \"ofcr\", \"GUI_CP_ROMOFF_RES\", \"cent\"";
        let strings = Strings::from_game("english", table);
        let res = ResourceModel::default();
        let mut n = Needs { slots: vec![(0, 7)], officers: 2, crew: 0, race: "fed".into() };
        assert_eq!(line(Some(&strings), &res, &n, None), "requires 7 deut, 2 ofcr");
        n.race = "romulan".into();
        assert_eq!(line(Some(&strings), &res, &n, None), "requires 7 deut, 2 cent");
        n.race = "é".into();
        assert_eq!(line(Some(&strings), &res, &n, None), "requires 7 deut, 2 ofcr", "a non-ASCII race name does not panic");
    }

    #[test]
    fn the_missing_resource_is_named_from_the_sims_own_test() {
        let db = db(120, 5, "x");
        let poor = feed(&db, 30, [0, 0]);
        assert_eq!(shortfall(&poor, station(&db)), Some(Shortfall::Resource(0)));
        let n = needs(&db, station(&db)).unwrap();
        let text = line(None, &ResourceModel::default(), &n, shortfall(&poor, station(&db)));
        assert_eq!(text, "needs: 120 dilithium, 5 officers (short of: dilithium)");
        let rich = feed(&db, 500, [0, 0]);
        assert_eq!(shortfall(&rich, station(&db)), None);
        let capped = feed(&db, 500, [0, 3]);
        assert_eq!(shortfall(&capped, station(&db)), Some(Shortfall::Officers));
    }

    #[test]
    fn the_build_buttons_carry_the_line_in_both_tooltips() {
        let db = db(120, 5, "x");
        let feed = feed(&db, 30, [0, 0]);
        let slots = ButtonSlots::from_header("int BUTTON_LOCATION_BACK = 5;\n").0;
        let buttons = control::buttons(&feed, Page::Build, &slots, None);
        let item = buttons.iter().find(|b| matches!(b.act, Act::Build { .. })).expect("a build item");
        let line = "needs: 120 dilithium, 5 officers (short of: dilithium)";
        assert!(item.tip.ends_with(&format!("({line})")), "{}", item.tip);
        assert!(item.verbose.as_deref().is_some_and(|v| v.ends_with(line)), "{:?}", item.verbose);
    }

    #[test]
    fn the_refusal_names_the_resource_on_armada_one_only() {
        let db = db(120, 0, "x");
        let feed = feed(&db, 30, [0, 0]);
        let key = station(&db);
        assert_eq!(refusal(&feed, Some(&UiProfile::Armada1), None, key).as_deref(), Some("Cannot build: not enough dilithium"));
        assert_eq!(refusal(&feed, Some(&UiProfile::Armada2), None, key), None, "Armada II keeps its wording");
        assert_eq!(refusal(&feed, Some(&UiProfile::Lcars), None, key), None);
        let rich = self::feed(&db, 500, [0, 0]);
        assert_eq!(refusal(&rich, Some(&UiProfile::Armada1), None, key), None, "affordable: no refusal");
    }

    /// A research station (id 2) whose list is one pod item, plus the cruiser: authored costs.
    fn research_db(cost: [f64; 4], officers: f64) -> Arc<ContentDb> {
        let fake = sta_content::fake::db();
        let mut b = ContentDb::builder().profile(fake.profile()).features(fake.features().clone());
        for w in fake.weapons() {
            b = b.weapon(w.clone());
        }
        for u in fake.units() {
            let mut u = u.clone();
            if u.odf == "fx_station" {
                u.build.list = vec![sta_content::DefRef::new("fx_pod")];
            }
            if u.odf == "fx_cruiser" {
                let mut pod = u.clone();
                pod.odf = "fx_pod".into();
                pod.build.list = Vec::new();
                pod.build.cost = cost.iter().map(|c| Fixed::from_f64(*c)).collect();
                pod.build.officers = Fixed::from_f64(officers);
                b = b.unit(pod);
            }
            b = b.unit(u);
        }
        Arc::new(b.build().unwrap())
    }

    fn research_feed(db: &Arc<ContentDb>, stock: i32, officers: [i32; 2]) -> HudFeed {
        let mut source = ScriptedSource::from_json5(
            "{ units: [{ id: 2, def: 'fx_station', owner: 1, hull: 100, rails: [{ at: 0, pos: { x: 0, y: 0, z: 0 } }] }] }",
            db,
        )
        .unwrap();
        let mut f = feed(db, stock, officers);
        let mut view = (*source.step(&[]).view).clone();
        view.players = f.view.players.clone();
        view.players[0].resources = vec![Fixed::from_int(stock); 4];
        f.view = Arc::new(view);
        f.selected = vec![EntityId(2)];
        f
    }

    #[test]
    fn a_research_item_gets_the_line_and_a_fractional_cost_shows_what_the_sim_charges() {
        let db = research_db([0.5, 1.9, 2.0, 0.0], 0.0);
        let feed = research_feed(&db, 500, [0, 0]);
        let slots = ButtonSlots::from_header("int BUTTON_LOCATION_BACK = 5;\n").0;
        let buttons = control::buttons(&feed, Page::Build, &slots, None);
        let item = buttons.iter().find(|b| matches!(b.act, Act::Build { .. })).expect("a research pod item");
        // Rounded up to the next whole unit; the zero biomatter slot is not listed.
        assert!(item.tip.ends_with("(needs: 1 dilithium, 2 latinum, 2 metal)"), "{}", item.tip);
        assert_eq!(whole(0.5), 1);
        assert_eq!(whole(1.9), 2);
        assert_eq!(whole(2.0), 2);
    }

    #[test]
    fn a_zero_cost_slot_beside_a_positive_one_is_left_out() {
        let db = research_db([0.0, 6.0, 0.0, 0.0], 0.0);
        let n = needs(&db, db.key("fx_pod").unwrap()).unwrap();
        assert_eq!(n.slots, vec![(1, 6)]);
        assert_eq!(line(None, &ResourceModel::default(), &n, None), "needs: 6 latinum");
    }

    #[test]
    fn the_officers_shortfall_reads_through_line_and_refusal() {
        let db = research_db([0.0, 0.0, 0.0, 0.0], 5.0);
        let key = db.key("fx_pod").unwrap();
        let capped = research_feed(&db, 500, [0, 3]);
        assert_eq!(shortfall(&capped, key), Some(Shortfall::Officers));
        let n = needs(&db, key).unwrap();
        assert_eq!(line(None, &ResourceModel::default(), &n, Some(Shortfall::Officers)), "needs: 5 officers (short of: officers)");
        assert_eq!(refusal(&capped, Some(&UiProfile::Armada1), None, key).as_deref(), Some("Cannot build: not enough officers"));
    }

    #[test]
    fn a_table_without_the_keys_falls_back_and_never_prints_empty_brackets() {
        let db = db(120, 5, "x");
        let feed = feed(&db, 500, [0, 0]);
        let slots = ButtonSlots::from_header("int BUTTON_LOCATION_BACK = 5;\n").0;
        // A table that has none of the Control Panel parts: the English fallbacks, no panic.
        let bare = Strings::from_game("english", "\"UNRELATED_KEY\", \"x\"");
        let buttons = control::buttons(&feed, Page::Build, &slots, Some(&bare));
        let item = buttons.iter().find(|b| matches!(b.act, Act::Build { .. })).unwrap();
        assert!(item.tip.ends_with("(needs: 120 dilithium, 5 officers)"), "{}", item.tip);
        assert!(!item.tip.contains("()") && !item.tip.contains("GUI_CP_"), "{}", item.tip);
        // A table that blanks the bracket words: no empty brackets either.
        let blank = Strings::from_game("english", "\"GUI_CP_START_EXTRA\", \"\", \"GUI_CP_END_EXTRA\", \"\"");
        let buttons = control::buttons(&feed, Page::Build, &slots, Some(&blank));
        let item = buttons.iter().find(|b| matches!(b.act, Act::Build { .. })).unwrap();
        assert!(!item.tip.contains("()") && item.tip.contains("needs: 120 dilithium"), "{}", item.tip);
        // And the words with an unrelated table fall back to the resource model's own.
        let n = needs(&db, station(&db)).unwrap();
        assert_eq!(line(Some(&bare), &ResourceModel::default(), &n, None), "needs: 120 dilithium, 5 officers");
    }

    #[test]
    fn armada_two_and_the_generic_hud_keep_their_build_tooltips() {
        let db = db(120, 5, "x");
        let feed = feed(&db, 30, [0, 0]);
        // The Armada II stock palette's build buttons (`stock::buttons`) and the generic grid.
        let stock = crate::ui::hud::stock::buttons(&feed, Page::Build, &Default::default());
        let generic = crate::ui::hud::model::grid(&feed, Page::Build);
        for list in [&stock, &generic] {
            let item = list.iter().find(|b| matches!(b.act, Act::Build { .. })).expect("a build item");
            assert!(!item.tip.contains("needs:") && !item.tip.contains("short of"), "{}", item.tip);
            assert!(item.verbose.as_deref().is_none_or(|v| !v.contains("needs:")), "{:?}", item.verbose);
        }
        assert_eq!(stock.iter().find(|b| matches!(b.act, Act::Build { .. })).unwrap().tip, "Build fx_station");
    }
}
