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
//! - Build item, short: the class's `tooltip` text, the colon string, a space and the `hotkeyLabel` text
//!   when the item has one, then a line with the cost: an opening bracket, entries "amount, space, the
//!   first letter of the resource's name" apart by a separator, a closing bracket. Resources at or
//!   below zero are left out; the order is dilithium, metal, latinum, biomatter, crew, officers. Verbose:
//!   the same first line, the class's `verboseTooltip` text when it has one, then the cost line with the
//!   full names. The bracket, separator and name keys are `GUI_CP_START_EXTRA`, `GUI_CP_AMOUNT_SEPARATE`,
//!   `GUI_CP_END_EXTRA` and `GUI_CP_*_RES` (crew and the race's officer word included): read from the
//!   executable, static; the composition measured at one window size on four build items. STA64's own:
//!   the colon is the world tooltips' string, an item with no cost has no cost line, a name or
//!   bracket the table lacks shows as a literal, and the officer word follows the builder's race.
//! - Hotkey markers ([`hotkey_text`]): measured at one window size; the bound-key rule is read from the
//!   executable, static.
//!
//! STA64's own reading: without a `tooltip` key a non-craft shows the class name (the original shows its
//! object name; whether that is the same text is not determined). Planets are hoverable units here and take
//! the craft form; nebulae and moons are not hoverable units and show no tooltip (gap).
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


/// The hotkey marker of the original's tooltip strings: the character after it is drawn underlined
/// and the marker itself is not drawn (measured at one window size, in a command's name and again
/// inside its angle brackets).
pub const HOTKEY_MARK: char = '^';

/// The string-table keys of the cost line (`GUI_CP_START_EXTRA`, `GUI_CP_AMOUNT_SEPARATE`,
/// `GUI_CP_END_EXTRA`): the opening bracket, the separator between entries and the closing bracket.
/// Read from the executable, static; the composition measured at one window size on four build items.
pub const COST_START_KEY: &str = "GUI_CP_START_EXTRA";
pub const COST_SEPARATE_KEY: &str = "GUI_CP_AMOUNT_SEPARATE";
pub const COST_END_KEY: &str = "GUI_CP_END_EXTRA";
/// Used when the table lacks the key: STA64's own.
pub const COST_START_FALLBACK: &str = "(";
pub const COST_SEPARATE_FALLBACK: &str = "/";
pub const COST_END_FALLBACK: &str = ")";

/// Put the key the player has bound into a hotkey slot of `text`, then consume the markers.
///
/// Slot rule (read from the executable, static): a single character directly before a `>`, that
/// follows a `<` (with the marker between them or not) and is not preceded by a letter or digit,
/// is the key; `bound` replaces it, upper-cased. Without a binding (`None`) the string's own letter
/// stays. Markers: each [`HOTKEY_MARK`] is removed and the character after it is reported by its
/// index in the returned text, for the caller to draw underlined; a marker at the end is removed.
/// Text without a marker or slot comes back unchanged with no indices.
pub fn hotkey_text(text: &str, bound: Option<char>) -> (String, Vec<usize>) {
    let mut chars: Vec<char> = text.chars().collect();
    if let Some(key) = bound.filter(|c| c.is_ascii_alphanumeric()) {
        for j in 1..chars.len().saturating_sub(1) {
            if chars[j + 1] != '>' || matches!(chars[j], '<' | '>') || chars[j - 1].is_alphanumeric() {
                continue;
            }
            let opened = chars[j - 1] == '<' || (chars[j - 1] == HOTKEY_MARK && j >= 2 && chars[j - 2] == '<');
            if opened {
                chars[j] = key.to_ascii_uppercase();
            }
        }
    }
    let mut out = String::with_capacity(text.len());
    let mut marks = Vec::new();
    let mut underline_next = false;
    let mut n = 0usize;
    for c in chars {
        if c == HOTKEY_MARK && !underline_next {
            underline_next = true;
            continue;
        }
        if underline_next {
            marks.push(n);
            underline_next = false;
        }
        out.push(c);
        n += 1;
    }
    (out, marks)
}

/// One entry of a build item's cost line.
#[derive(Clone, Debug, PartialEq)]
pub struct CostPart {
    /// The string-table key of the resource's name (`GUI_CP_DILITHIUM_RES`, ...).
    pub key: &'static str,
    /// The name used when the table lacks the key: STA64's own.
    pub fallback: String,
    pub amount: f32,
}

/// What a build item's tooltip is made of: the definition's keys and costs, before any key is resolved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildTipParts {
    /// The class's `tooltip` key (or text).
    pub tooltip: String,
    /// The class's `verboseTooltip` key.
    pub verbose: Option<String>,
    /// The class's `hotkeyLabel` key.
    pub hotkey: Option<String>,
    /// The costs in the fixed resource order (dilithium, metal, latinum, biomatter, crew, officers).
    pub cost: Vec<CostPart>,
}

/// What a button's tooltip takes from the definitions under the stock look, beyond its own text.
#[derive(Clone, Debug, PartialEq)]
pub enum StockTip {
    /// A build item: class text, hotkey label and cost line.
    Build(BuildTipParts),
    /// A resource template with two native `%d` number slots (crew stock and capacity).
    TwoNumbers(i64, i64),
    /// Fleet Operations system detail: class tooltip followed by its authored verbose text.
    ClassDetails,
    /// An individual meter or subsystem, with live values.
    Readout(crate::ui::hud::readouts::ReadoutText),
}

pub fn class_details(short: &str, verbose: Option<&str>, look: Lookup) -> String {
    let short = look(short);
    let Some(verbose) = verbose else { return short };
    let verbose = look(verbose);
    if short.is_empty() || verbose.is_empty() { return String::new() }
    format!("{short}{}\n{verbose}", separator(look, TOOLTIP_SEPARATE_KEY, TOOLTIP_SEPARATOR_FALLBACK))
}

/// Localize first, then substitute the two numeric slots in a native resource template.
pub fn two_numbers(key: &str, first: i64, second: i64, look: Lookup) -> String {
    let text = look(key);
    text.replacen("%d", &first.to_string(), 1).replacen("%d", &second.to_string(), 1)
}

fn first_line(p: &BuildTipParts, look: Lookup) -> String {
    let mut line = look(&p.tooltip);
    line.push_str(&separator(look, TOOLTIP_SEPARATE_KEY, TOOLTIP_SEPARATOR_FALLBACK));
    // A label the table cannot resolve is left out rather than shown as a key.
    if let Some(label) = p.hotkey.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        let text = look(label);
        if text != label {
            line.push(' ');
            line.push_str(&text);
        }
    }
    line
}

/// The cost line: the opening bracket, entries `amount name` (the name's first letter when `full` is
/// false) apart by the separator with a space each side, the closing bracket. Resources at or below
/// zero are left out; no entry at all gives no line (STA64's own reading: free items show none).
fn cost_line(p: &BuildTipParts, look: Lookup, full: bool) -> Option<String> {
    let entries: Vec<String> = p
        .cost
        .iter()
        .filter(|c| c.amount > 0.0)
        .map(|c| {
            let name = look(c.key);
            let name = if name == c.key { c.fallback.clone() } else { name };
            let name = if full { name } else { name.chars().next().map(String::from).unwrap_or_default() };
            format!("{} {name}", c.amount as i64)
        })
        .collect();
    if entries.is_empty() {
        return None;
    }
    let sep = format!(" {} ", separator(look, COST_SEPARATE_KEY, COST_SEPARATE_FALLBACK));
    Some(format!(
        "{} {} {}",
        separator(look, COST_START_KEY, COST_START_FALLBACK),
        entries.join(&sep),
        separator(look, COST_END_KEY, COST_END_FALLBACK)
    ))
}

/// A build item's short tooltip: the class text, a colon and the hotkey label's text, then the cost
/// line with the first letter of each resource.
pub fn build_short(p: &BuildTipParts, look: Lookup) -> String {
    let mut out = first_line(p, look);
    if let Some(cost) = cost_line(p, look, false) {
        out.push('\n');
        out.push_str(&cost);
    }
    out
}

/// A build item's verbose tooltip: the same first line, the class's verbose text when it has one, then
/// the cost line with the full resource names.
pub fn build_verbose(p: &BuildTipParts, look: Lookup) -> String {
    let mut out = first_line(p, look);
    if let Some(v) = key(&p.verbose) {
        out.push('\n');
        out.push_str(&look(v));
    }
    if let Some(cost) = cost_line(p, look, true) {
        out.push('\n');
        out.push_str(&cost);
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

    #[test]
    fn native_two_number_template_localizes_before_substitution_and_preserves_empty_text() {
        let look = |key: &str| match key {
            "CREW" => "Crew: %d/%d".to_owned(),
            "EMPTY" => String::new(),
            other => other.to_owned(),
        };
        assert_eq!(two_numbers("CREW", 7, 12, &look), "Crew: 7/12");
        assert_eq!(two_numbers("EMPTY", 7, 12, &look), "");
    }

    #[test]
    fn fleet_ops_class_details_resolve_each_authored_key_at_display_time() {
        let look = |key: &str| match key {
            "SHORT" => "Class".to_owned(),
            "VERBOSE" => "Long description".to_owned(),
            "GUI_CP_TOOLTIP_SEPARATE" => "!".to_owned(),
            other => other.to_owned(),
        };
        assert_eq!(class_details("SHORT", Some("VERBOSE"), &look), "Class!\nLong description");
        assert_eq!(class_details("SHORT", None, &look), "Class");
        let blank = |key: &str| if key == "VERBOSE" { String::new() } else { look(key) };
        assert_eq!(class_details("SHORT", Some("VERBOSE"), &blank), "");
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

    // ----- hotkey slots and markers -----

    #[test]
    fn the_marker_is_consumed_and_the_next_character_is_reported_in_the_name_and_the_brackets() {
        assert_eq!(hotkey_text("^Zap <^Z>", None), ("Zap <Z>".to_owned(), vec![0, 5]));
        // A marker mid-word, two on one line, one at the end (dropped).
        assert_eq!(hotkey_text("a^bc d^e^", None), ("abc de".to_owned(), vec![1, 5]));
        // Text without a marker or slot is untouched.
        assert_eq!(hotkey_text("Plain tip: <F2>", None), ("Plain tip: <F2>".to_owned(), vec![]));
        assert_eq!(hotkey_text("", Some('Q')), (String::new(), vec![]));
        // The second line of a verbose text counts its characters from the start of the whole text.
        assert_eq!(hotkey_text("x\n^y", None), ("x\ny".to_owned(), vec![2]));
    }

    #[test]
    fn the_bound_key_fills_the_slot_and_the_own_letter_stays_without_a_binding() {
        assert_eq!(hotkey_text("^Zap <^Z>", Some('q')), ("Zap <Q>".to_owned(), vec![0, 5]));
        assert_eq!(hotkey_text("Zap <Z>", Some('7')), ("Zap <7>".to_owned(), vec![]));
        assert_eq!(hotkey_text("Zap <^Z>: Does zapping.", None).0, "Zap <Z>: Does zapping.");
        // A slot after a letter or digit (a key name such as F2), several characters, or no opening bracket are left alone.
        assert_eq!(hotkey_text("Go <F2>", Some('q')).0, "Go <F2>");
        assert_eq!(hotkey_text("Go <ab>", Some('q')).0, "Go <ab>");
        assert_eq!(hotkey_text("Go a>", Some('q')).0, "Go a>");
        // Only a letter or digit key is a character; another key leaves the string's letter.
        assert_eq!(hotkey_text("Go <G>", Some('+')).0, "Go <G>");
    }

    // ----- build items -----

    fn build_table(key: &str) -> String {
        match key {
            "K_TIP" => "Zed Class".into(),
            "K_VERB" => "A long\nparagraph.".into(),
            "K_LABEL" => "<F9>".into(),
            "GUI_CP_TOOLTIP_SEPARATE" => "!".into(),
            "GUI_CP_START_EXTRA" => "[".into(),
            "GUI_CP_AMOUNT_SEPARATE" => "|".into(),
            "GUI_CP_END_EXTRA" => "]".into(),
            "RES_D" => "dust".into(),
            "RES_M" => "mud".into(),
            "RES_L" => "lace".into(),
            "RES_B" => "bog".into(),
            "RES_C" => "hands".into(),
            "RES_O" => "bosses".into(),
            other => other.into(),
        }
    }

    fn part(key: &'static str, amount: f32) -> CostPart {
        CostPart { key, fallback: format!("fb-{key}"), amount }
    }

    fn build_parts() -> BuildTipParts {
        BuildTipParts {
            tooltip: "K_TIP".into(),
            verbose: Some("K_VERB".into()),
            hotkey: Some("K_LABEL".into()),
            cost: vec![part("RES_D", 100.0), part("RES_M", 0.0), part("RES_L", 80.0), part("RES_B", 0.0), part("RES_C", 6.0), part("RES_O", 2.0)],
        }
    }

    #[test]
    fn a_build_items_short_text_is_the_class_text_a_colon_the_hotkey_and_the_cost_line_with_first_letters() {
        assert_eq!(build_short(&build_parts(), &build_table), "Zed Class! <F9>\n[ 100 d | 80 l | 6 h | 2 b ]");
    }

    #[test]
    fn the_verbose_text_has_the_same_first_line_the_description_and_the_cost_line_with_full_names() {
        assert_eq!(build_verbose(&build_parts(), &build_table), "Zed Class! <F9>\nA long\nparagraph.\n[ 100 dust | 80 lace | 6 hands | 2 bosses ]");
    }

    #[test]
    fn a_build_item_without_a_hotkey_label_or_a_verbose_text_leaves_them_out() {
        let p = BuildTipParts { hotkey: None, verbose: None, ..build_parts() };
        assert_eq!(build_short(&p, &build_table), "Zed Class!\n[ 100 d | 80 l | 6 h | 2 b ]");
        assert_eq!(build_verbose(&p, &build_table), "Zed Class!\n[ 100 dust | 80 lace | 6 hands | 2 bosses ]");
        // A label the table cannot resolve is left out, not shown as a key.
        let unresolved = BuildTipParts { hotkey: Some("NO_SUCH_LABEL".into()), ..build_parts() };
        assert!(build_short(&unresolved, &build_table).starts_with("Zed Class!\n"));
    }

    #[test]
    fn zero_costs_are_omitted_the_order_is_the_lists_and_no_cost_gives_no_line() {
        let only_crew = BuildTipParts { cost: vec![part("RES_D", 0.0), part("RES_C", 4.0)], ..build_parts() };
        assert_eq!(build_short(&only_crew, &build_table), "Zed Class! <F9>\n[ 4 h ]");
        let free = BuildTipParts { cost: vec![part("RES_D", 0.0), part("RES_C", -1.0)], ..build_parts() };
        assert_eq!(build_short(&free, &build_table), "Zed Class! <F9>");
        assert_eq!(build_verbose(&free, &build_table), "Zed Class! <F9>\nA long\nparagraph.");
    }

    #[test]
    fn missing_table_keys_fall_back_to_stas64s_own_literals() {
        let bare = |k: &str| k.to_owned();
        let p = BuildTipParts { hotkey: None, ..build_parts() };
        assert_eq!(
            build_short(&p, &bare),
            format!("K_TIP{TOOLTIP_SEPARATOR_FALLBACK}\n{COST_START_FALLBACK} 100 f {COST_SEPARATE_FALLBACK} 80 f {COST_SEPARATE_FALLBACK} 6 f {COST_SEPARATE_FALLBACK} 2 f {COST_END_FALLBACK}")
        );
        let v = build_verbose(&p, &bare);
        assert!(v.ends_with(&format!("{COST_START_FALLBACK} 100 fb-RES_D {COST_SEPARATE_FALLBACK} 80 fb-RES_L {COST_SEPARATE_FALLBACK} 6 fb-RES_C {COST_SEPARATE_FALLBACK} 2 fb-RES_O {COST_END_FALLBACK}")), "{v}");
    }
}
