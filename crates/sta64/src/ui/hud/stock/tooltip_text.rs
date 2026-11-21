//! What the Armada II stock tooltip says over a world object: the short and the verbose
//! composition. Pure functions over resolved strings; the callers hand in the string table.
//!
//! Rules (grades per item):
//! - Short: `[craft name + separator] + class name`, the separator being the key's string with one
//!   space on each side (read from the executable, static). A craft with no name gives the class name
//!   alone. An object that is not a craft and whose class has a `tooltip` key gives that key's
//!   text instead; without the key it gives the class name. Structure read from the executable,
//!   static; the five frames of the original show the craft form and the one with a `tooltip`
//!   key (seen in frames, measured).
//! - Verbose: `[craft name + separator] + (the class's `tooltip` key text, else the class name)`;
//!   when the class has a `verboseTooltip` key, then the colon string, a line break and that key's
//!   text. Structure read from the executable, static; measured on five frames.
//! - The class name is the class's `unitName`; `unitName`, `tooltip`, `verboseTooltip` and the two
//!   separators (`GUI_CP_CLASS_SEPARATE`, `GUI_CP_TOOLTIP_SEPARATE`) are string-table keys,
//!   resolved when the tooltip is shown. Read from data; the lookup order from the executable, static.
//! - One colour for the whole box, no per-line colour. Read from the executable, static.
//!
//! STA64's own, where the original gives nothing to follow: a key the table lacks shows as it is
//! written (a literal authored caption), and a separator the table lacks falls back to
//! [`CLASS_SEPARATOR_FALLBACK`] / [`TOOLTIP_SEPARATOR_FALLBACK`] (no install, a mod without the keys).

/// The string-table key of the separator between a craft's name and its class text (a space each side is added).
pub const CLASS_SEPARATE_KEY: &str = "GUI_CP_CLASS_SEPARATE";
/// The string-table key of the string between the class text and the verbose paragraph.
pub const TOOLTIP_SEPARATE_KEY: &str = "GUI_CP_TOOLTIP_SEPARATE";
/// Used when the table has no entry for [`CLASS_SEPARATE_KEY`]: STA64's own.
pub const CLASS_SEPARATOR_FALLBACK: &str = "-";
/// Used when the table has no entry for [`TOOLTIP_SEPARATE_KEY`]: STA64's own.
pub const TOOLTIP_SEPARATOR_FALLBACK: &str = ":";

/// What a world object's class and name give the tooltip, before any key is resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldTipParts {
    /// The object is a craft (the name part applies; a `tooltip` key does not replace the class
    /// name in the short form).
    pub craft: bool,
    /// The craft's name, when it has one.
    pub name: Option<String>,
    /// The class's `unitName` (a key or a literal), else STA64's fallback caption.
    pub class_name: String,
    /// The class's `tooltip` key.
    pub tooltip: Option<String>,
    /// The class's `verboseTooltip` key.
    pub verbose: Option<String>,
}

/// A string-table lookup: the text of a key, the key itself when the table has none.
pub type Lookup<'a> = &'a dyn Fn(&str) -> String;

fn separator(look: Lookup, key: &str, fallback: &str) -> String {
    let text = look(key);
    if text == key { fallback.to_owned() } else { text }
}

fn name_part(p: &WorldTipParts, look: Lookup) -> String {
    match p.name.as_deref().map(str::trim).filter(|n| p.craft && !n.is_empty()) {
        Some(name) => format!("{name} {} ", separator(look, CLASS_SEPARATE_KEY, CLASS_SEPARATOR_FALLBACK)),
        None => String::new(),
    }
}

fn key(k: &Option<String>) -> Option<&str> {
    k.as_deref().map(str::trim).filter(|k| !k.is_empty())
}

/// The short composition.
pub fn world_short(p: &WorldTipParts, look: Lookup) -> String {
    let mut out = name_part(p, look);
    match key(&p.tooltip).filter(|_| !p.craft) {
        Some(k) => out.push_str(&look(k)),
        None => out.push_str(&look(&p.class_name)),
    }
    out
}

/// The verbose composition.
pub fn world_verbose(p: &WorldTipParts, look: Lookup) -> String {
    let mut out = name_part(p, look);
    match key(&p.tooltip) {
        Some(k) => out.push_str(&look(k)),
        None => out.push_str(&look(&p.class_name)),
    }
    if let Some(v) = key(&p.verbose) {
        out.push_str(&separator(look, TOOLTIP_SEPARATE_KEY, TOOLTIP_SEPARATOR_FALLBACK));
        out.push('\n');
        out.push_str(&look(v));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An authored table: keys to texts, anything else shows as written.
    fn table(key: &str) -> String {
        match key {
            "K_CLASS" => "Alpha Class".into(),
            "K_TIP" => "Alpha Class Mk II".into(),
            "K_VERB" => "A long\nparagraph.".into(),
            "GUI_CP_CLASS_SEPARATE" => "~".into(),
            "GUI_CP_TOOLTIP_SEPARATE" => "!".into(),
            other => other.into(),
        }
    }

    fn parts() -> WorldTipParts {
        WorldTipParts { craft: true, name: Some("Hull 7".into()), class_name: "K_CLASS".into(), tooltip: Some("K_TIP".into()), verbose: Some("K_VERB".into()) }
    }

    #[test]
    fn a_named_craft_gives_name_separator_class_in_the_short_form() {
        assert_eq!(world_short(&parts(), &table), "Hull 7 ~ Alpha Class");
    }

    #[test]
    fn the_verbose_form_swaps_the_class_for_the_tooltip_text_and_adds_a_paragraph() {
        assert_eq!(world_verbose(&parts(), &table), "Hull 7 ~ Alpha Class Mk II!\nA long\nparagraph.");
    }

    #[test]
    fn a_craft_without_a_name_gives_the_class_alone() {
        let p = WorldTipParts { name: None, ..parts() };
        assert_eq!(world_short(&p, &table), "Alpha Class");
        assert_eq!(world_verbose(&p, &table), "Alpha Class Mk II!\nA long\nparagraph.");
        let blank = WorldTipParts { name: Some("  ".into()), ..parts() };
        assert_eq!(world_short(&blank, &table), "Alpha Class");
    }

    #[test]
    fn a_class_without_the_keys_falls_back_to_the_class_name_and_has_no_paragraph() {
        let p = WorldTipParts { tooltip: None, verbose: None, ..parts() };
        assert_eq!(world_short(&p, &table), "Hull 7 ~ Alpha Class");
        assert_eq!(world_verbose(&p, &table), "Hull 7 ~ Alpha Class");
        let empty = WorldTipParts { tooltip: Some(" ".into()), verbose: Some(String::new()), ..parts() };
        assert_eq!(world_verbose(&empty, &table), "Hull 7 ~ Alpha Class");
    }

    #[test]
    fn a_non_craft_shows_its_tooltip_text_in_the_short_form_and_never_a_name() {
        let p = WorldTipParts { craft: false, ..parts() };
        assert_eq!(world_short(&p, &table), "Alpha Class Mk II");
        assert_eq!(world_verbose(&p, &table), "Alpha Class Mk II!\nA long\nparagraph.");
        let bare = WorldTipParts { tooltip: None, ..p };
        assert_eq!(world_short(&bare, &table), "Alpha Class");
    }

    #[test]
    fn separators_the_table_lacks_fall_back() {
        let bare = |k: &str| match k {
            "K_VERB" => "More.".to_owned(),
            other => other.to_owned(),
        };
        assert_eq!(world_short(&parts(), &bare), format!("Hull 7 {CLASS_SEPARATOR_FALLBACK} K_CLASS"));
        assert_eq!(world_verbose(&parts(), &bare), format!("Hull 7 {CLASS_SEPARATOR_FALLBACK} K_TIP{TOOLTIP_SEPARATOR_FALLBACK}\nMore."));
    }
}
