//! Cosmetic craft-name selection. Never advances the simulation RNG or changes
//! rules, entity state, save formats or command/view serialization.

use sta_content::UnitDef;
use sta_sim::EntityId;

pub(super) fn unit_name(def: &UnitDef) -> Option<&str> {
    def.raw.str("unitName").map(str::trim).filter(|name| !name.is_empty())
}

pub(super) fn instance(def: &UnitDef, id: EntityId) -> String {
    let names = list(def.raw.raw("possibleCraftNames").unwrap_or_default());
    if names.is_empty() {
        return unit_name(def).unwrap_or(&def.odf).to_owned();
    }
    // FNV-1a plus SplitMix64's finalizer: explicit wrapping integer operations
    // give all clients the same pseudorandom choice for this entity identity.
    // Do not use RandomState/DefaultHasher, platform usize, or gameplay RNG.
    let mut key = def.odf.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    }) ^ u64::from(id.0);
    key = (key ^ (key >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    key = (key ^ (key >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    key ^= key >> 31;
    names[(key % names.len() as u64) as usize].to_owned()
}

/// Quoted items are whole names, not whitespace-separated words. ODF parsing
/// already removes comments and joins continuation rows before inheritance.
fn list(raw: &str) -> Vec<&str> {
    let mut remaining = raw.trim();
    let mut names = Vec::new();
    while !remaining.is_empty() {
        if let Some(quoted) = remaining.strip_prefix('"') {
            let Some(end) = quoted.find('"') else { break };
            let name = &quoted[..end];
            if !name.trim().is_empty() {
                names.push(name);
            }
            remaining = &quoted[end + 1..];
        } else {
            let end = remaining.find(|c: char| c.is_whitespace() || c == ';').unwrap_or(remaining.len());
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
        assert_eq!(list("\"USS O'Toole\" \"USS M.L.King, Jr.\" \"Étoile = // 2\""),
            ["USS O'Toole", "USS M.L.King, Jr.", "Étoile = // 2"]);
        assert_eq!(list("\"\" \"  \" \"USS Last\";"), ["USS Last"]);
        assert!(list("\"unterminated").is_empty());
    }

    #[test]
    fn fallback_uses_authored_type_then_odf() {
        let unit = def("unitName = \"Construction Ship\"\npossibleCraftNames = \"\"\n");
        assert_eq!(instance(&unit, EntityId(42)), "Construction Ship");
        assert_eq!(unit_name(&unit), Some("Construction Ship"));
        assert_eq!(instance(&def("unitName = \"\""), EntityId(42)), "fx_constructor");
    }

    #[test]
    fn name_pool_inherits_and_child_replaces_the_whole_list() {
        let parent = "possibleCraftNames =\n \"USS First\" \"USS Second\"\n \"USS Third\"\nunitName = \"Construction Ship\"";
        let mut unit = def("");
        unit.raw = Arc::new(sta_odf::resolve("child", &mut |name| match name {
            "child.odf" => Some("#include \"parent\"\n".to_owned()),
            "parent.odf" => Some(parent.to_owned()),
            _ => None,
        }).unwrap());
        let names = list(unit.raw.raw("possibleCraftNames").unwrap());
        assert_eq!(names, ["USS First", "USS Second", "USS Third"]);
        let choices: std::collections::BTreeSet<_> = (1..=64).map(|id| instance(&unit, EntityId(id))).collect();
        assert_eq!(choices.len(), 3);
        let a = instance(&unit, EntityId(1));
        assert_eq!(instance(&unit.clone(), EntityId(1)), a);
        unit.raw = Arc::new(sta_odf::resolve("child", &mut |name| match name {
            "child.odf" => Some("#include \"parent\"\npossibleCraftNames = \"USS Override\"".to_owned()),
            "parent.odf" => Some(parent.to_owned()),
            _ => None,
        }).unwrap());
        assert_eq!(instance(&unit, EntityId(1)), "USS Override");
    }

    #[test]
    fn installed_name_pools_are_read_without_splitting_craft_names() {
        let Ok(root) = std::env::var("STA64_TEST_INSTALL") else {
            eprintln!("skip installed name pools: STA64_TEST_INSTALL is unset");
            return;
        };
        let active_mod = std::env::var("STA64_TEST_MOD").ok();
        let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), active_mod.as_deref()).unwrap();
        let (index, _) = sta_content::OdfIndex::build(&vfs);
        let (db, _) = sta_content::ContentDb::load(&vfs, &index).unwrap();
        let mut checked = 0;
        for unit in db.units() {
            let Some(raw) = unit.raw.raw("possibleCraftNames").filter(|v| !v.trim().is_empty()) else { continue };
            let names = list(raw);
            assert!(!names.is_empty(), "{} has a nonempty but unreadable pool", unit.odf);
            for id in [1, 2, 42, u32::MAX] {
                assert!(names.contains(&instance(unit, EntityId(id)).as_str()), "{} entity {id}", unit.odf);
            }
            if unit.odf == "fconst" {
                eprintln!("fconst: unitName={:?}, {} complete craft names; entity 1={}", unit_name(unit), names.len(), instance(unit, EntityId(1)));
            }
            checked += 1;
        }
        assert!(checked > 0, "the requested install contains no nonempty craft-name pools");
        eprintln!("{checked} installed craft-name pools checked");
    }
}
