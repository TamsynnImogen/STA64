//! The resource model of a game: which stock slots exist, what each is called
//! and which ODF keys feed it.
//!
//! The engine of every Armada game has the same four stock slots, ids 0..3:
//! dilithium, latinum, metal and biomatter. The ODF keys are fixed by the
//! slot (`<slot>Cost`, `normal<slot>`, `amt<slot>`, `recycle<slot>`), but the
//! words a game shows for a slot are its own, and so is which slots a race
//! uses (`noLatinum`, `noMetal`, ... in its `gui_<race>.cfg`):
//!
//! | game | dilithium | latinum | metal | biomatter |
//! |---|---|---|---|---|
//! | Armada II, STA2 Classic | Dilithium | Latinum | Metal | Bio-matter (Species 8472) |
//! | Fleet Operations 3.x, Dominion War, Fleet Ops 4.0 | Dilithium | **Tritanium** | Collective Connections (Borg) | **Supply** |
//! | Fleet Operations Roots (its own ODFs) | Dilithium | Connection Nodes (Borg) | **Tritanium** | **Supply** (Biomatter: Species 8472) |
//! | Armada I | Dilithium only | | | |
//!
//! The words are read from the install's string table
//! (`GUI_RD_<SLOT>_VTOOLTIP`, the text before the colon), so a mod that
//! renames a resource is followed; [`ResourceModel::for_profile`] is the
//! fallback when the table has none.

use sta_odf::Odf;

use crate::features::Profile;
use crate::strings::StringTable;

/// Stock slots per player.
pub const SLOTS: usize = 4;

/// The slot ids.
pub const DILITHIUM: usize = 0;
pub const LATINUM: usize = 1;
pub const METAL: usize = 2;
pub const BIOMATTER: usize = 3;

/// The slot names of the engine, lower case: the stems of the ODF keys
/// (`dilithiumCost`, `normallatinum`, `amtmetal`) and the scripts' and
/// the bench's resource names.
pub const KEYS: [&str; SLOTS] = ["dilithium", "latinum", "metal", "biomatter"];

/// The key of a slot's cost in a unit ODF (`dilithiumCost`).
pub fn cost_key(slot: usize) -> String {
    format!("{}Cost", KEYS[slot])
}

/// The key of a slot's amount in a resource node's ODF (`amtmetal`).
pub fn amount_key(slot: usize) -> String {
    format!("amt{}", KEYS[slot])
}

/// The slot a resource node's ODF fills: the first slot (in id order) whose
/// `amt<slot>` is positive. Fleet Operations 4.0's tritanium moon holds
/// `amtlatinum` and `amtbiomatter`, Roots' holds `amtmetal` and
/// `amtbiomatter`, a dilithium moon `amtdilithium` and `amtbiomatter`.
pub fn node_slot(odf: &Odf) -> Option<usize> {
    (0..SLOTS).find(|&s| odf.f32(&amount_key(s)).is_some_and(|v| v > 0.0))
}

/// What a game calls its stock slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceModel {
    names: [String; SLOTS],
    /// Words other than the names that mean a slot in a node's `resource`
    /// text (stock names on a profile whose table renames them).
    aliases: Vec<(String, usize)>,
}

impl Default for ResourceModel {
    fn default() -> Self {
        ResourceModel::for_profile(Profile::Native)
    }
}

impl ResourceModel {
    /// A model with these slot names.
    pub fn new(names: [&str; SLOTS]) -> ResourceModel {
        ResourceModel { names: names.map(str::to_owned), aliases: Vec::new() }
    }

    /// The names of a profile's games. Fleet Operations 4 is the 3.x layout:
    /// the latinum slot is the tritanium, the biomatter slot the supply.
    pub fn for_profile(profile: Profile) -> ResourceModel {
        match profile {
            Profile::Fo4 => ResourceModel::new(["Dilithium", "Tritanium", "Collective Connections", "Supply"]),
            // Roots' own ODFs name tritanium for the metal; the aliases keep an
            // install whose table is missing readable.
            Profile::Native | Profile::Sta2Classic | Profile::Sta1Classic => {
                let mut m = ResourceModel::new(["Dilithium", "Latinum", "Metal", "Biomatter"]);
                m.aliases = vec![("tritanium".into(), METAL), ("supply".into(), BIOMATTER)];
                m
            }
        }
    }

    /// The profile's model with the install's own words on top: for each slot
    /// the text before the colon of `GUI_RD_<SLOT>_VTOOLTIP`.
    pub fn from_strings(profile: Profile, table: &StringTable) -> ResourceModel {
        let mut m = ResourceModel::for_profile(profile);
        for (slot, key) in KEYS.iter().enumerate() {
            let tip = table.get(&format!("GUI_RD_{}_VTOOLTIP", key.to_ascii_uppercase()));
            if let Some(name) = tip.and_then(first_word) {
                m.names[slot] = name;
            }
        }
        m
    }

    /// The aliases as the content digest renders them, `["tritanium":2,...]`
    /// in their own order. The slot names are display text and not part of it.
    pub fn aliases_text(&self) -> String {
        let items: Vec<String> = self.aliases.iter().map(|(w, s)| format!("{}:{s}", crate::digest::quoted(w))).collect();
        format!("[{}]", items.join(","))
    }

    /// The word the game shows for `slot`; `"?"` past the last.
    pub fn name(&self, slot: usize) -> &str {
        self.names.get(slot).map_or("?", String::as_str)
    }

    /// The slot a resource word means (a node's `resource` text, a script's
    /// name): the engine's slot names and the game's own, any case, found
    /// anywhere in the text. Latinum, metal and biomatter are tried before
    /// dilithium, which is also the answer for text that names none of them
    /// (`None` here, so the caller chooses).
    pub fn slot_of_text(&self, text: &str) -> Option<usize> {
        let t = text.to_ascii_lowercase();
        let has = |w: &str| !w.is_empty() && t.contains(&w.to_ascii_lowercase());
        // The game's own words first: on Fleet Operations "tritanium" is the latinum slot.
        for s in [LATINUM, METAL, BIOMATTER, DILITHIUM] {
            if has(&self.names[s]) {
                return Some(s);
            }
        }
        for s in [LATINUM, METAL, BIOMATTER, DILITHIUM] {
            if has(KEYS[s]) {
                return Some(s);
            }
        }
        self.aliases.iter().find(|(w, _)| has(w)).map(|&(_, s)| s)
    }
}

/// The text of a tooltip before its colon, when that is one short name.
fn first_word(tip: &str) -> Option<String> {
    let (head, _) = tip.split_once(':')?;
    let head = head.trim();
    (!head.is_empty() && head.len() <= 32 && head.chars().all(|c| c.is_ascii_alphabetic() || c == ' ' || c == '-')).then(|| head.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn odf(text: &str) -> Odf {
        sta_odf::resolve("n", &mut |_| Some(text.to_owned())).unwrap()
    }

    #[test]
    fn cost_and_amount_keys_follow_the_slot() {
        assert_eq!([0, 1, 2, 3].map(cost_key), ["dilithiumCost", "latinumCost", "metalCost", "biomatterCost"]);
        assert_eq!(amount_key(METAL), "amtmetal");
    }

    #[test]
    fn a_node_fills_its_first_amt_slot() {
        // Fleet Ops 4.0's tritanium moon: the latinum slot, with supply beside it.
        assert_eq!(node_slot(&odf("amtlatinum = 25000\namtbiomatter = 25000\n")), Some(LATINUM));
        // Roots' own: the metal slot.
        assert_eq!(node_slot(&odf("amtmetal = 25000\namtbiomatter = 25000\n")), Some(METAL));
        assert_eq!(node_slot(&odf("amtdilithium = 25000\namtbiomatter = 25000\n")), Some(DILITHIUM));
        assert_eq!(node_slot(&odf("amtdilithium = 0\namtmetal = 5\n")), Some(METAL));
        assert_eq!(node_slot(&odf("isinfinite = 1\n")), None);
    }

    #[test]
    fn profiles_name_their_slots() {
        let names = |p| (0..SLOTS).map(|s| ResourceModel::for_profile(p).name(s).to_owned()).collect::<Vec<_>>();
        assert_eq!(names(Profile::Sta2Classic), ["Dilithium", "Latinum", "Metal", "Biomatter"]);
        assert_eq!(names(Profile::Fo4)[..2], ["Dilithium", "Tritanium"]);
        assert_eq!(names(Profile::Fo4)[3], "Supply");
    }

    #[test]
    fn the_string_table_renames_slots() {
        let t = StringTable::parse(
            "\"GUI_RD_LATINUM_VTOOLTIP\", \"Connections Nodes: Borg\",\n\"GUI_RD_METAL_VTOOLTIP\", \"Tritanium: stuff, more\",\n\"GUI_RD_BIOMATTER_VTOOLTIP\", \"no colon here\",\n",
        );
        let m = ResourceModel::from_strings(Profile::Sta2Classic, &t);
        assert_eq!((m.name(LATINUM), m.name(METAL), m.name(BIOMATTER), m.name(DILITHIUM)), ("Connections Nodes", "Tritanium", "Biomatter", "Dilithium"));
    }

    #[test]
    fn node_text_means_the_slot_of_the_games_word() {
        let stock = ResourceModel::for_profile(Profile::Sta2Classic);
        assert_eq!(stock.slot_of_text("latinum"), Some(LATINUM));
        assert_eq!(stock.slot_of_text("map_tritanium_moon_res"), Some(METAL));
        assert_eq!(stock.slot_of_text("map_dilithium_moon_res"), Some(DILITHIUM));
        let fo = ResourceModel::for_profile(Profile::Fo4);
        assert_eq!(fo.slot_of_text("map_tritanium_moon_res"), Some(LATINUM));
        assert_eq!(fo.slot_of_text("map_biomatter_moon_res"), Some(BIOMATTER));
        assert_eq!(fo.slot_of_text("nothing"), None);
    }
}
