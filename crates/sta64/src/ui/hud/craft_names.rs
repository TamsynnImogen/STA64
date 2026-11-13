//! Cosmetic craft-name selection. Never advances the simulation RNG or changes
//! rules, entity state, save formats or command/view serialization.

use sta_content::UnitDef;
use sta_sim::EntityId;

pub(super) fn unit_name(def: &UnitDef) -> Option<&str> {
    def.raw
        .str("unitName")
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

pub(super) fn instance(def: &UnitDef, id: EntityId) -> String {
    identity(def, id).name
}

pub(super) struct Identity {
    pub name: String,
    pub captain: Option<String>,
    pub registry: Option<String>,
}

pub(super) fn identity(def: &UnitDef, id: EntityId) -> Identity {
    // Keep source row indices even though blank craft names are not selectable.
    let names = rows(def.raw.raw("possibleCraftNames").unwrap_or_default());
    let selected = names
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, name)| !name.trim().is_empty())
        .collect::<Vec<_>>();
    let Some((row, name)) = choose(def, id, &selected) else {
        return Identity {
            name: unit_name(def).unwrap_or(&def.odf).to_owned(),
            captain: None,
            registry: None,
        };
    };
    Identity {
        name: name.to_owned(),
        captain: companion(def, "possibleCaptainNames", row),
        registry: companion(def, "possibleCraftRegistry", row),
    }
}

fn choose<'a>(def: &UnitDef, id: EntityId, names: &[(usize, &'a str)]) -> Option<(usize, &'a str)> {
    if names.is_empty() {
        return None;
    }
    // FNV-1a plus SplitMix64's finalizer: explicit wrapping integer operations
    // give all clients the same pseudorandom choice for this entity identity.
    // Do not use RandomState/DefaultHasher, platform usize, or gameplay RNG.
    let mut key = def
        .odf
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        })
        ^ u64::from(id.0);
    key = (key ^ (key >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    key = (key ^ (key >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    key ^= key >> 31;
    Some(names[(key % names.len() as u64) as usize])
}

fn companion(def: &UnitDef, key: &str, row: usize) -> Option<String> {
    rows(def.raw.raw(key).unwrap_or_default())
        .get(row)
        .copied()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

/// Quoted items are whole names, not whitespace-separated words. ODF parsing
/// already removes comments and joins continuation rows before inheritance.
#[cfg(test)]
fn list(raw: &str) -> Vec<&str> {
    rows(raw)
        .into_iter()
        .filter(|name| !name.trim().is_empty())
        .collect()
}

/// Parse row entries without dropping intentionally blank quoted values.
fn rows(raw: &str) -> Vec<&str> {
    let mut remaining = raw.trim();
    let mut names = Vec::new();
    while !remaining.is_empty() {
        if let Some(quoted) = remaining.strip_prefix('"') {
            let Some(end) = quoted.find('"') else { break };
            let name = &quoted[..end];
            names.push(name);
            remaining = &quoted[end + 1..];
        } else {
            let end = remaining
                .find(|c: char| c.is_whitespace() || c == ';')
                .unwrap_or(remaining.len());
            if end > 0 {
                names.push(&remaining[..end]);
            }
            remaining = &remaining[end..];
        }
        remaining = remaining.trim_start_matches(|c: char| c.is_whitespace() || c == ';');
    }
    names
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn def(text: &str) -> UnitDef {
        let mut unit = UnitDef::new("fx_constructor", "constructionrig");
        unit.raw = Arc::new(sta_odf::resolve("unit", &mut |_| Some(text.to_owned())).unwrap());
        unit
    }

    #[test]
    fn quoted_names_preserve_spaces_case_and_punctuation() {
        assert_eq!(
            list("\"USS O'Toole\" \"USS M.L.King, Jr.\" \"Étoile = // 2\""),
            ["USS O'Toole", "USS M.L.King, Jr.", "Étoile = // 2"]
        );
        assert_eq!(list("\"\" \"  \" \"USS Last\";"), ["USS Last"]);
        assert!(list("\"unterminated").is_empty());
    }

    #[test]
    fn fallback_uses_authored_type_then_odf() {
        let unit = def("unitName = \"Construction Ship\"\npossibleCraftNames = \"\"\n");
        assert_eq!(instance(&unit, EntityId(42)), "Construction Ship");
        assert_eq!(unit_name(&unit), Some("Construction Ship"));
        assert_eq!(
            instance(&def("unitName = \"\""), EntityId(42)),
            "fx_constructor"
        );
    }

    #[test]
    fn name_pool_inherits_and_child_replaces_the_whole_list() {
        let parent = "possibleCraftNames =\n \"USS First\" \"USS Second\"\n \"USS Third\"\nunitName = \"Construction Ship\"";
        let mut unit = def("");
        unit.raw = Arc::new(
            sta_odf::resolve("child", &mut |name| match name {
                "child.odf" => Some("#include \"parent\"\n".to_owned()),
                "parent.odf" => Some(parent.to_owned()),
                _ => None,
            })
            .unwrap(),
        );
        let names = list(unit.raw.raw("possibleCraftNames").unwrap());
        assert_eq!(names, ["USS First", "USS Second", "USS Third"]);
        let choices: std::collections::BTreeSet<_> =
            (1..=64).map(|id| instance(&unit, EntityId(id))).collect();
        assert_eq!(choices.len(), 3);
        let a = instance(&unit, EntityId(1));
        assert_eq!(instance(&unit.clone(), EntityId(1)), a);
        unit.raw = Arc::new(
            sta_odf::resolve("child", &mut |name| match name {
                "child.odf" => {
                    Some("#include \"parent\"\npossibleCraftNames = \"USS Override\"".to_owned())
                }
                "parent.odf" => Some(parent.to_owned()),
                _ => None,
            })
            .unwrap(),
        );
        assert_eq!(instance(&unit, EntityId(1)), "USS Override");
    }

    #[test]
    fn duplicate_names_and_blank_craft_rows_keep_companion_source_rows() {
        let unit = def(
            "possibleCraftNames = \"Duplicate\" \"\" \"Duplicate\"\npossibleCaptainNames = \"First\" \"Blank row\" \"Third\"\npossibleCraftRegistry = \"NCC-1\" \"NCC-blank\" \"NCC-3\"",
        );
        for id in 1..=64 {
            let selected =
                choose(&unit, EntityId(id), &[(0, "Duplicate"), (2, "Duplicate")]).unwrap();
            let actual = identity(&unit, EntityId(id));
            assert_eq!(actual.name, "Duplicate");
            assert_eq!(
                actual.captain.as_deref(),
                Some(if selected.0 == 0 { "First" } else { "Third" })
            );
            assert_eq!(
                actual.registry.as_deref(),
                Some(if selected.0 == 0 { "NCC-1" } else { "NCC-3" })
            );
        }
    }

    #[test]
    fn short_and_blank_companion_rows_do_not_shift_or_wrap() {
        let unit = def(
            "possibleCraftNames = \"First\" \"Second\" \"Third\"\npossibleCaptainNames = \"Captain 1\" \"\"\npossibleCraftRegistry = \"Registry 1\"",
        );
        for id in 1..=64 {
            let selected = choose(
                &unit,
                EntityId(id),
                &[(0, "First"), (1, "Second"), (2, "Third")],
            )
            .unwrap();
            let actual = identity(&unit, EntityId(id));
            assert_eq!(
                actual.captain.as_deref(),
                match selected.0 {
                    0 => Some("Captain 1"),
                    1 | 2 => None,
                    _ => unreachable!(),
                }
            );
            assert_eq!(
                actual.registry.as_deref(),
                (selected.0 == 0).then_some("Registry 1")
            );
        }
    }

    #[test]
    fn existing_name_choices_are_pinned_and_companions_preserve_authored_text() {
        let unit = def(
            "possibleCraftNames = \"First\" \"Second\" \"Third\"\npossibleCaptainNames = \"  Élise  \" \"Second captain\" \"Third captain\"\npossibleCraftRegistry = \"R-1\" \"R-2\" \"R-3\"",
        );
        // Choices from STA64's pre-companion name selection for this ODF/id.
        for (id, expected) in [
            (1, "Second"),
            (2, "Third"),
            (17, "First"),
            (42, "Third"),
            (u32::MAX, "Third"),
        ] {
            assert_eq!(instance(&unit, EntityId(id)), expected);
        }
        assert_eq!(
            identity(&unit, EntityId(17)).captain.as_deref(),
            Some("  Élise  ")
        );
    }

    #[test]
    fn companion_lists_inherit_and_explicit_empty_child_lists_override() {
        let mut unit = def("");
        unit.raw = Arc::new(sta_odf::resolve("child", &mut |name| match name {
            "child.odf" => Some("#include \"parent\"\npossibleCaptainNames = \"\"".to_owned()),
            "parent.odf" => Some("possibleCraftNames = \"Ship\"\npossibleCaptainNames = \"Inherited Captain\"\npossibleCraftRegistry = \"Inherited Registry\"".to_owned()),
            _ => None,
        }).unwrap());
        let actual = identity(&unit, EntityId(1));
        assert_eq!(actual.name, "Ship");
        assert_eq!(actual.captain, None);
        assert_eq!(actual.registry.as_deref(), Some("Inherited Registry"));
    }

    #[test]
    fn fallback_name_has_no_companions_and_choice_is_stable() {
        let unit = def(
            "unitName = \"Fallback\"\npossibleCraftNames = \"\"\npossibleCaptainNames = \"Captain\"\npossibleCraftRegistry = \"Registry\"",
        );
        let first = identity(&unit, EntityId(17));
        assert_eq!(first.name, "Fallback");
        assert_eq!(first.captain, None);
        assert_eq!(first.registry, None);
        assert_eq!(identity(&unit, EntityId(17)).name, first.name);
    }

    #[test]
    fn installed_name_pools_are_read_without_splitting_craft_names() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
            eprintln!("skip installed name pools: STA64_TEST_INSTALL is unset");
            return;
        };
        let active_mod = std::env::var("STA64_TEST_MOD").ok();
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), active_mod.as_deref())
            .unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let (db, _) = sta_content::ContentDb::load(&vfs, &index).unwrap();
        let mut checked = 0;
        for unit in db.units() {
            let Some(raw) = unit
                .raw
                .raw("possibleCraftNames")
                .filter(|v| !v.trim().is_empty())
            else {
                continue;
            };
            let names = list(raw);
            assert!(
                !names.is_empty(),
                "{} has a nonempty but unreadable pool",
                unit.odf
            );
            for id in [1, 2, 42, u32::MAX] {
                assert!(
                    names.contains(&instance(unit, EntityId(id)).as_str()),
                    "{} entity {id}",
                    unit.odf
                );
            }
            if unit.odf == "fconst" {
                eprintln!(
                    "fconst: unitName={:?}, {} complete craft names; entity 1={}",
                    unit_name(unit),
                    names.len(),
                    instance(unit, EntityId(1))
                );
            }
            checked += 1;
        }
        assert!(
            checked > 0,
            "the requested install contains no nonempty craft-name pools"
        );
        eprintln!("{checked} installed craft-name pools checked");
    }
}
