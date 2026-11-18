//! SIM-44: a unit's `fleetCapFile` is a capacity list the build gate applies like the XML tech
//! tree's `<capacitylist>`, where no XML list of that name exists.

use sta_content::{Availability, ContentDb, Overrides, Owned, Profile, TechTree, Unmet, odf::load_with};

const CAP: &str = "fleetCapClass0 = \"cruiser\"\nfleetCapClass0CapValue = 2\nfleetCapClass1 = \"frigate\"\nmaxCap = 4\ncapTooltip = \"Cruisers ( %d / %d )\"\n";

fn load(cap: &str) -> ContentDb {
    let files = [
        ("yard", "classLabel = \"shipyard\"\nbuildItem0 = \"cruiser\"\nbuildItem1 = \"frigate\"\n".to_owned()),
        ("cruiser", "classLabel = \"craft\"\nfleetCapFile = \"capfile.odf\"\n".to_owned()),
        ("frigate", "classLabel = \"craft\"\nfleetCapFile = \"capfile.odf\"\n".to_owned()),
        ("capfile", cap.to_owned()),
    ];
    let stems: Vec<String> = ["yard", "cruiser", "frigate"].iter().map(|s| s.to_string()).collect();
    let (db, warnings) = load_with(Profile::Fo4, &Overrides::default(), &stems, &mut |name, version| {
        (version == 0).then(|| files.iter().find(|(s, _)| name.eq_ignore_ascii_case(&format!("{s}.odf"))).map(|(_, t)| t.clone())).flatten()
    })
    .unwrap();
    assert!(warnings.iter().all(|w| !w.message.contains("fleetCap")), "{warnings:?}");
    db
}

fn owning(cruisers: u32, frigates: u32) -> Owned {
    Owned::new("fx").with("cruiser", cruisers).with("frigate", frigates)
}

#[test]
fn the_cap_file_is_read_as_a_capacity_list() {
    let db = load(CAP);
    let cap = db.unit(db.key("cruiser").unwrap()).unwrap().fleet_cap.clone().unwrap();
    assert_eq!(cap.name, "capfile");
    assert_eq!(cap.max, 4);
    assert_eq!(cap.members, [("cruiser".to_owned(), 2), ("frigate".to_owned(), 1)], "capValue defaults to 1");
    assert_eq!(cap.tooltip, "Cruisers ( %d / %d )");
    // Both units share the one list.
    assert!(std::sync::Arc::ptr_eq(&cap, db.unit(db.key("frigate").unwrap()).unwrap().fleet_cap.as_ref().unwrap()));
}

#[test]
fn the_build_gate_refuses_once_the_weighted_count_reaches_the_cap() {
    let db = load(CAP);
    let tree = TechTree::default();
    let gate = |o: &Owned, what: &str| tree.availability_for_builder(&db, what, "yard", o, false);
    // 1 cruiser (2) + 1 frigate (1) = 3 < 4: room.
    assert_eq!(gate(&owning(1, 1), "cruiser"), Availability::Available);
    // 2 cruisers = 4: full, for every member.
    for what in ["cruiser", "frigate"] {
        assert_eq!(gate(&owning(2, 0), what), Availability::Locked { unmet: vec![Unmet::CapacityFull("capfile".into())], hidden: false }, "{what}");
    }
}

#[test]
fn an_xml_list_of_the_same_name_governs_and_no_max_means_no_limit() {
    // The XML tree defines `capfile` itself: the ODF's maxCap 4 is not applied a second time.
    let db = load(CAP);
    let (tree, _) = TechTree::parse("t.xml", "<techtree><capacitylist name=\"capfile\" maxCapValue=\"100\"><obj capValue=\"1\">cruiser.odf</obj></capacitylist></techtree>");
    assert!(tree.capacity_list("capfile").is_some());
    assert_ne!(tree.availability_for_builder(&db, "cruiser", "yard", &owning(5, 0), false), Availability::Locked { unmet: vec![Unmet::CapacityFull("capfile".into())], hidden: false });
    // A cap file without maxCap (FO's veteran slots rely on the XML list): no limit of its own.
    let db = load(&CAP.replace("maxCap = 4\n", ""));
    assert_eq!(TechTree::default().availability_for_builder(&db, "cruiser", "yard", &owning(50, 0), false), Availability::Available);
}

/// Install-gated (STA64_TEST_INSTALL + STA64_TEST_MOD, empty for none; skipped otherwise): every
/// unit that names a cap file gets its list, and the lists with no XML counterpart carry a limit.
#[test]
fn installed_cap_files_load() {
    let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else { return };
    let vfs = sta_vfs::Vfs::mount_install(std::path::Path::new(&root), Some(m.as_str()).filter(|m| !m.is_empty())).unwrap();
    let (index, _) = sta_content::OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load(&vfs, &index).unwrap();
    let named = db.units().filter(|u| u.raw.str("fleetCapFile").is_some()).count();
    let capped: Vec<_> = db.units().filter_map(|u| u.fleet_cap.as_ref()).collect();
    let mut names: Vec<&str> = capped.iter().map(|c| c.name.as_str()).collect();
    names.sort();
    names.dedup();
    let xml: Vec<&str> = names.iter().copied().filter(|n| db.tech_tree().is_some_and(|t| t.capacity_list(n).is_some())).collect();
    eprintln!("{named} units name a cap file, {} carry one; {} lists, {} also in the XML tree", capped.len(), names.len(), xml.len());
    assert_eq!(named, capped.len(), "every named cap file loads");
    for n in &names {
        if !xml.contains(n) && capped.iter().any(|c| c.name == *n && c.max == i64::MAX) {
            eprintln!("note: {n} has no maxCap and no XML list: no limit");
        }
    }
}
