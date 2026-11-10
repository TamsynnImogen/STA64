//! UI-09: every user-visible UI string goes through here.
//!
//! The English text at the call site is the key (`tr("Single player")`), and
//! doubles as the fallback, so the UI reads correctly with no install, with a
//! mod that lacks a key, and in tests. A [`Strings`] table maps that key to
//! the active language's text. It is filled from two layers, later wins:
//!
//! 1. the game's own `Dynamic_Localized_Strings.h` (`"GAME_KEY", "text"`),
//!    reached through [`GAME_KEYS`], which maps STA64 call-site keys to the
//!    game's key where the original has one;
//! 2. `sta64_<language>.strings` in the mounted install or mod, tab-separated
//!    `key<TAB>text` lines, for everything the game never had and for
//!    translations.
//!
//! The text helpers are called from plain builder functions that have no
//! `World`, so the active table lives behind a process-wide lock that the
//! [`Strings`] resource keeps in step. Changing [`Language`] reloads it.
//!
//! The lint test at the bottom fails when a UI source file hands a string
//! literal straight to a text-spawning helper instead of through [`tr`].

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use bevy::prelude::*;
use sta_vfs::Vfs;

/// STA64 call-site key -> the game's own string key, for the strings the
/// original has. A game key missing from the install falls back to the
/// English call-site text.
pub const GAME_KEYS: &[(&str, &str)] = &[(
    "Double click to return to strategic mode.",
    "GUI_CV_MIN_TOOLTIP",
)];

/// The active UI language, as the lower-case name its files use (`english`).
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct Language(pub String);

impl Default for Language {
    fn default() -> Self {
        Language("english".into())
    }
}

/// The loaded table for the active [`Language`].
#[derive(Resource, Debug, Default, Clone)]
pub struct Strings {
    pub language: String,
    map: HashMap<String, String>,
}

fn active() -> &'static RwLock<HashMap<String, String>> {
    static T: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

impl Strings {
    /// Reads the layers above from `vfs`; a missing file contributes nothing.
    pub fn load(vfs: &Vfs, language: &str) -> Strings {
        let read = |p: &str| {
            vfs.read(p)
                .and_then(Result::ok)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        };
        let game = read("Dynamic_Localized_Strings.h")
            .map(|t| parse_game(&t))
            .unwrap_or_default();
        let mut map = HashMap::new();
        for (key, game_key) in GAME_KEYS {
            if let Some(text) = game.get(*game_key) {
                map.insert((*key).to_owned(), text.clone());
            }
        }
        if let Some(t) = read(&format!("sta64_{language}.strings")) {
            map.extend(parse_pairs(&t));
        }
        Strings {
            language: language.to_owned(),
            map,
        }
    }

    #[cfg(test)]
    /// A table straight from pairs (tests, mods built in code).
    pub fn from_pairs(language: &str, pairs: &[(&str, &str)]) -> Strings {
        Strings {
            language: language.to_owned(),
            map: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }

    /// Makes this the table [`tr`] reads.
    pub fn activate(&self) {
        if let Ok(mut g) = active().write() {
            *g = self.map.clone();
        }
    }
}

/// `"KEY",<ws>"text"` lines of `Dynamic_Localized_Strings.h`.
fn parse_game(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let mut q = line.trim().split('"');
        if let (Some(""), Some(k), Some(_), Some(v)) = (q.next(), q.next(), q.next(), q.next()) {
            out.insert(k.to_owned(), v.to_owned());
        }
    }
    out
}

fn parse_pairs(text: &str) -> impl Iterator<Item = (String, String)> + '_ {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
}

/// The text for `key` in the active language; the key itself (the English
/// default) when the table has no entry.
pub fn tr(key: &str) -> String {
    active()
        .read()
        .ok()
        .and_then(|g| g.get(key).cloned())
        .unwrap_or_else(|| key.to_owned())
}

/// [`tr`] with `{0}`, `{1}`... replaced by `args`, for strings with values in.
pub fn trf(key: &str, args: &[&dyn std::fmt::Display]) -> String {
    let mut s = tr(key);
    for (i, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{i}}}"), &a.to_string());
    }
    s
}

/// Reloads the table when [`Language`] changes. The host mounts the install as [`crate::viewer::Mounted`]; with
/// no install the table stays empty.
fn reload(
    lang: Res<Language>,
    vfs: Option<Res<crate::viewer::Mounted>>,
    mut strings: ResMut<Strings>,
) {
    if !lang.is_changed() && strings.language == lang.0 {
        return;
    }
    *strings = match vfs {
        Some(v) => Strings::load(&v.0, &lang.0),
        None => Strings {
            language: lang.0.clone(),
            ..default()
        },
    };
    strings.activate();
}

pub struct I18nPlugin;

impl Plugin for I18nPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Language>()
            .init_resource::<Strings>()
            .add_systems(PreUpdate, reload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_the_key_and_formats() {
        assert_eq!(tr("Zzz unlikely key"), "Zzz unlikely key");
        assert_eq!(trf("Team {0} of {1}", &[&2, &"x"]), "Team 2 of x");
    }

    #[test]
    fn parses_game_and_pair_files() {
        let g = parse_game("\t\"GUI_A\",\t\"Hello there\",\n// x\n\t\"GUI_B\", \"b\"");
        assert_eq!(g["GUI_A"], "Hello there");
        assert_eq!(g["GUI_B"], "b");
        let p: Vec<_> = parse_pairs("# c\nHold\tHalten\n").collect();
        assert_eq!(p, vec![("Hold".to_owned(), "Halten".to_owned())]);
    }

    #[test]
    fn language_switch_reloads_the_table() {
        let mut app = App::new();
        app.add_plugins(I18nPlugin);
        app.update();
        app.world_mut().resource_mut::<Language>().0 = "german".into();
        // No install: the table reloads empty for the new language.
        app.update();
        assert_eq!(app.world().resource::<Strings>().language, "german");
        Strings::from_pairs("german", &[("Hold", "Halten")]).activate();
        assert_eq!(tr("Hold"), "Halten");
        Strings::default().activate();
        assert_eq!(tr("Hold"), "Hold");
    }

    // ---- Lint: no literal UI strings -------------------------------------

    /// Helpers whose string arguments are shown to the player. A literal (or a
    /// `format!`) handed to one of these directly is a violation; wrap it in
    /// [`tr`] / [`trf`]. Empty literals are fine (placeholders filled later).
    const SINKS: &[&str] = &[
        "Text::new",
        "frame",
        "text",
        "label",
        "button",
        "disabled_button",
        "toggle",
        "cell",
        "cell_label",
        "panel",
        "bar_row",
        "Notice::new",
        "Tooltip::new",
        "verbose",
    ];

    /// Files that may keep literals, and why.
    const ALLOW_FILES: &[&str] = &[
        // The widget gallery is a developer showcase (`--ui-gallery`), not game UI.
        "ui/theme/gallery.rs",
        // Shared test modules that are whole files.
        "tests.rs",
        // This file's own lint source text.
        "ui/i18n.rs",
    ];

    fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                rs_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    /// Top-level arguments of the call whose `(` is at `open`.
    fn args(src: &str, open: usize) -> Vec<&str> {
        let b = src.as_bytes();
        let (mut depth, mut start, mut out, mut i, mut in_str) =
            (0i32, open + 1, vec![], open, false);
        while i < b.len() {
            let c = b[i];
            if in_str {
                if c == b'\\' {
                    i += 1;
                } else if c == b'"' {
                    in_str = false;
                }
            } else {
                match c {
                    b'"' => in_str = true,
                    b'(' | b'[' | b'{' => depth += 1,
                    b')' | b']' | b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            out.push(src[start..i].trim());
                            return out;
                        }
                    }
                    b',' if depth == 1 => {
                        out.push(src[start..i].trim());
                        start = i + 1;
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        out
    }

    fn violations(src: &str) -> Vec<String> {
        // Production code only: everything before the test module.
        let src = src.split("#[cfg(test)]").next().unwrap_or(src);
        let mut bad = vec![];
        for sink in SINKS {
            let pat = format!("{sink}(");
            for (at, _) in src.match_indices(&pat) {
                let before = src[..at].chars().next_back();
                let method = *sink == "verbose";
                // Whole identifiers only; `.label(` methods and `fn` definitions are not sinks.
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                if (!method && before == Some('.')) || src[..at].trim_end().ends_with("fn") {
                    continue;
                }
                let all = args(src, at + sink.len());
                // `panel("id")` in the HUD layout is a lookup by name, not text.
                if *sink == "panel" && all.len() < 2 {
                    continue;
                }
                for a in all {
                    let a = a.trim_start_matches('&');
                    // `format!("{}%", n)`-style numeric formats carry no words.
                    let numeric = a.starts_with("format!")
                        && !a
                            .split('"')
                            .nth(1)
                            .unwrap_or("")
                            .split(['{', '}'])
                            .step_by(2)
                            .any(|s| s.chars().any(char::is_alphabetic));
                    if ((a.starts_with('"') && a != "\"\"") || a.starts_with("format!")) && !numeric
                    {
                        bad.push(format!("{sink}({a}"));
                    }
                }
            }
        }
        bad
    }

    #[test]
    fn lint_detects_a_literal() {
        assert_eq!(
            violations("fn f() { button(&t, \"Go\", 1.0); }"),
            vec!["button(\"Go\""]
        );
        assert!(violations("fn f() { button(&t, &tr(\"Go\"), 1.0); text(\"\", 1.0); }").is_empty());
        assert_eq!(
            violations("fn f() { Notice::new(S, format!(\"x {a}\")); }").len(),
            1
        );
    }

    #[test]
    fn no_literal_ui_strings() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![];
        rs_files(&root.join("ui"), &mut files);
        rs_files(&root.join("frontend"), &mut files);
        let mut report = vec![];
        for f in files {
            let rel = f
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if ALLOW_FILES.iter().any(|a| rel.ends_with(a)) {
                continue;
            }
            for v in violations(&std::fs::read_to_string(&f).unwrap()) {
                report.push(format!("{rel}: {v}"));
            }
        }
        assert!(
            report.is_empty(),
            "literal UI strings (wrap in i18n::tr / trf):\n{}",
            report.join("\n")
        );
    }
}
