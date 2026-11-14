//! Authored command button metadata. Capability and action policy live in the HUD model.

use std::collections::{BTreeMap, BTreeSet};

use bevy::{log::warn, prelude::Color};
use sta_assets::a2fmt::odf::Odf;
use sta_content::OdfIndex;
use sta_vfs::Vfs;

#[derive(Clone, Debug, Default)]
pub struct NativeCommands {
    /// Lower-case ODF stems to authored command records.
    pub commands: BTreeMap<String, NativeCommand>,
    /// Menu names, in stable lower-case order (and therefore stable IDs).
    pub groups: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct NativeCommand {
    pub odf: String,
    pub command: String,
    pub button: String,
    pub preferred: Option<[usize; 2]>,
    pub menu: Option<String>,
    pub tooltip: Option<String>,
    pub verbose: Option<String>,
    pub normal: Option<Color>,
    pub hovered: Option<Color>,
    pub pressed: Option<Color>,
    pub disabled: Option<Color>,
    pub active: Option<Color>,
}

impl NativeCommand {
    pub fn sprite_names(&self) -> Vec<String> {
        let bare = if self
            .button
            .get(..2)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("b_"))
        {
            &self.button[2..]
        } else {
            &self.button
        };
        let prefixed = format!("b_{bare}");
        if prefixed.eq_ignore_ascii_case(bare) {
            vec![prefixed]
        } else {
            vec![prefixed, bare.to_owned()]
        }
    }
}

fn odf_stem(name: &str) -> String {
    let base = name.trim().replace('\\', "/");
    let base = base.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    base.strip_suffix(".odf").unwrap_or(&base).to_owned()
}

fn clean_str(odf: &Odf, key: &str) -> Option<String> {
    odf.get_str(key)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn numbers(raw: &str) -> Option<Vec<f64>> {
    raw.split(|c: char| c.is_whitespace() || matches!(c, ',' | '(' | ')' | '"'))
        .filter(|part| !part.is_empty())
        .map(|part| part.trim_end_matches(['f', 'F']).parse::<f64>().ok())
        .collect()
}

fn preferred(odf: &Odf) -> Option<[usize; 2]> {
    let parts = numbers(odf.get("preferredPosition")?)?;
    let [x, y, ..] = parts.as_slice() else {
        return None;
    };
    let valid = |n: f64| n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n < usize::MAX as f64;
    (valid(*x) && valid(*y)).then_some([*x as usize, *y as usize])
}

fn colour(odf: &Odf, key: &str) -> Option<Color> {
    let values = numbers(odf.get(key)?)?;
    if !(3..=4).contains(&values.len()) || !values.iter().all(|n| n.is_finite() && (0.0..=1.0).contains(n)) {
        return None;
    }
    Some(Color::srgba(
        values[0] as f32,
        values[1] as f32,
        values[2] as f32,
        values.get(3).copied().unwrap_or(1.0) as f32,
    ))
}

impl NativeCommands {
    pub fn load(vfs: &Vfs) -> Self {
        let (index, warnings) = OdfIndex::build(vfs);
        for warning in warnings {
            warn!("stock HUD command ODF index: {warning}");
        }
        let resolve =
            |name: &str| Odf::load_layered(name, |n, version| index.load(vfs, n, version).map(String::into_bytes)).ok();
        let mut names = Vec::new();
        if let Some(all) = resolve("allcommands.odf") {
            names.extend(all.get_strs("commandNames").into_iter().map(str::to_owned));
        } else {
            warn!("stock HUD: allcommands.odf is missing; using available command ODFs");
        }
        // The shared HUD implements these capabilities even when allcommands omits them.
        names.extend(["move", "attack", "stop", "rally", "harvest", "cancelbuild", "colonize"].map(str::to_owned));
        let mut commands = BTreeMap::new();
        let mut groups = BTreeSet::new();
        for name in names {
            let key = odf_stem(&name);
            if key.is_empty() || commands.contains_key(&key) {
                continue;
            }
            let file = format!("{key}.odf");
            let Some(odf) = resolve(&file) else {
                warn!("stock HUD: command ODF {file} is missing");
                continue;
            };
            if !odf.missing.is_empty() {
                warn!("stock HUD: unresolved includes in {file}: {:?}", odf.missing);
            }
            let menu = clean_str(&odf, "menu");
            if let Some(value) = &menu {
                groups.insert(value.to_ascii_lowercase());
            }
            commands.insert(
                key.clone(),
                NativeCommand {
                    odf: file,
                    command: clean_str(&odf, "commandName").unwrap_or_else(|| key.clone()),
                    button: clean_str(&odf, "buttonName").unwrap_or_else(|| key.clone()),
                    preferred: preferred(&odf),
                    menu,
                    tooltip: clean_str(&odf, "tooltip"),
                    verbose: clean_str(&odf, "verbose"),
                    normal: colour(&odf, "colorNormal"),
                    hovered: colour(&odf, "colorMouseOver"),
                    pressed: colour(&odf, "colorPressed"),
                    disabled: colour(&odf, "colorDisabled"),
                    active: colour(&odf, "colorActive"),
                },
            );
        }
        Self {
            commands,
            groups: groups.into_iter().collect(),
        }
    }

    /// Find by file stem or authored commandName, case insensitively.
    pub fn get(&self, name: &str) -> Option<&NativeCommand> {
        let key = odf_stem(name);
        self.commands.get(&key).or_else(|| {
            self.commands
                .values()
                .find(|command| command.command.eq_ignore_ascii_case(name))
        })
    }

    pub fn menu_id(&self, name: &str) -> Option<u32> {
        self.groups
            .iter()
            .position(|group| group.eq_ignore_ascii_case(name))
            .and_then(|id| u32::try_from(id).ok())
    }

    pub fn menu_name(&self, id: u32) -> Option<&str> {
        self.groups.get(id as usize).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Install(PathBuf);

    impl Install {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("sta64_stock_commands_{name}_{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }
        fn file(&self, path: &str, text: &str) -> &Self {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
            self
        }
        fn mount(&self, active: Option<&str>) -> Vfs {
            Vfs::mount_install(&self.0, active).unwrap()
        }
    }

    impl Drop for Install {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn layered_commands_keep_own_values_and_recursive_includes() {
        let install = Install::new("layered");
        install.file("Data/odf/commands/allcommands.odf", "commandNames =\n \"attack.odf\" \"move\"\n \"missing\"\n")
            .file("Data/odf/commands/attack.odf", "#include \"basebutton.odf\"\ncommandName = \"ATTACK\"\nbuttonName = \"attack_old\"\npreferredPosition = 1 2 0 0\n")
            .file("Data/odf/other/basebutton.odf", "tooltip = \"Base tooltip\"\ncolorNormal = 0.1 0.2 0.3\n")
            .file("Data/odf/commands/move.odf", "commandName = \"MOVE\"\nbuttonName = \"b_move\"\nmenu = \"Orders\"\n")
            .file("Data/Mods/Child/info.ini", "[Mod]\nName=Child\n")
            .file("Data/Mods/Child/odf/commands/attack.odf", "#include \"attack.odf\"\nbuttonName = \"attack_new\"\ncolorMouseOver = 0.2 0.4 0.6 0.8\nmenu = \"Combat\"\n");
        let loaded = NativeCommands::load(&install.mount(Some("Child")));
        let attack = loaded.get("ATTACK.ODF").unwrap();
        assert_eq!(attack.odf, "attack.odf");
        assert_eq!(attack.command, "ATTACK");
        assert_eq!(attack.tooltip.as_deref(), Some("Base tooltip"));
        assert_eq!(attack.button, "attack_new");
        assert_eq!(attack.preferred, Some([1, 2]));
        assert_eq!(attack.normal, Some(Color::srgba(0.1, 0.2, 0.3, 1.0)));
        assert_eq!(attack.hovered, Some(Color::srgba(0.2, 0.4, 0.6, 0.8)));
        assert_eq!(attack.sprite_names(), ["b_attack_new", "attack_new"]);
        assert_eq!(loaded.get("MOVE").unwrap().sprite_names(), ["b_move", "move"]);
        assert!(loaded.get("missing").is_none());
        assert_eq!(loaded.groups, ["combat", "orders"]);
        assert_eq!(loaded.menu_id("ORDERS"), Some(1));
        assert_eq!(loaded.menu_name(0), Some("combat"));
    }

    #[test]
    fn missing_allcommands_and_invalid_values_are_bounded() {
        let install = Install::new("fallback");
        install
            .file(
                "Data/odf/commands/stop.odf",
                "buttonName = \"stop\"\npreferredPosition = -1 2 0 0\ncolorNormal = 2 0 0\nmenu = \"\"\n",
            )
            .file(
                "Data/odf/commands/rally.odf",
                "preferredPosition = 0 3 0 0\ncolorActive = 0 1 0\n",
            );
        let loaded = NativeCommands::load(&install.mount(None));
        assert_eq!(loaded.commands.len(), 2);
        let stop = loaded.get("STOP").unwrap();
        assert_eq!(stop.preferred, None);
        assert_eq!(stop.normal, None);
        assert_eq!(stop.menu, None);
        assert_eq!(loaded.get("rally").unwrap().preferred, Some([0, 3]));
        assert_eq!(
            loaded.get("rally").unwrap().active,
            Some(Color::srgba(0.0, 1.0, 0.0, 1.0))
        );
        assert!(loaded.groups.is_empty());
    }
}
