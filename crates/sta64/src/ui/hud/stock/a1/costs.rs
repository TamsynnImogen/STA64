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
//! short and the verbose tooltip because the short one shows first). TODO: crew (`crewCost`
//! exists on the unit, the sim neither charges nor checks it, so it is not listed).
//!
//! The missing resource is named from `sta_sim::production::afford::build_shortfall`, the sim's
//! own payment test over the client's view; nothing here compares stocks itself.

use sta_content::{ContentDb, ResourceModel};
use sta_sim::ContentKey;
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
    /// The unit's race name, for the officers word.
    pub race: String,
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
            .map(|(i, c)| (i, c.to_f32() as i64))
            .collect(),
        officers: def.build.officers.to_f32().max(0.0) as i64,
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

/// The refusal notice for a build the player cannot pay for, Armada I only (the other profiles keep
/// their wording): `Cannot build: not enough dilithium`.
pub(in crate::ui::hud) fn refusal(feed: &HudFeed, profile: Option<&UiProfile>, strings: Option<&Strings>, key: ContentKey) -> Option<String> {
    if !super::is_a1(profile) {
        return None;
    }
    let short = shortfall(feed, key)?;
    let db = feed.content.as_deref()?;
    let needs = needs(db, key)?;
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
        HudFeed { view: Arc::new(view), player: PlayerId(1), selected: vec![EntityId(1)], content: Some(db.clone()) }
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
        let mut n = Needs { slots: vec![(0, 7)], officers: 2, race: "fed".into() };
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
}
