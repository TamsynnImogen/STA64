//! Own-authored assignments; no original game data.
use sta_odf::{LookupPolicy, Odf, resolve_layered_with_policy};

fn resolve(policy: LookupPolicy) -> Odf {
    resolve_layered_with_policy("ship", policy, &mut |name, version| {
        match (name, version) {
            ("ship.odf", 0) => Some("#include \"ship.odf\"\n#include \"base.odf\"\nKeyExtra = 11\n\"own.odf\" 0.5\nkey = 22\nKEY = 33\nmissingSuffix without equals\nvalidSuffix = yes\n".into()),
            ("ship.odf", 1) => Some("key = 44\nmissingSuffix = 55\n".into()),
            ("base.odf", 0) => Some("key = 66\nbase = 77\n".into()),
            _ => None,
        }
    }).unwrap()
}

#[test]
fn first_prefix_uses_own_file_order_and_keeps_provenance_and_rows_together() {
    let odf = resolve(LookupPolicy::PrefixFirst);
    assert_eq!(odf.i64("kEy"), Some(11));
    assert_eq!(odf.source("KEY").unwrap().line, 3);
    assert_eq!(odf.source("KEY").unwrap().version, 0);
    assert_eq!(odf.rows("key").collect::<Vec<_>>(), ["\"own.odf\" 0.5"]);
    assert!(odf.has_rows("key"));
    assert_eq!(odf.i64("missing"), Some(55)); // bare line without '=' cannot win
    assert_eq!(odf.source("missing").unwrap().version, 1);
    assert_eq!(odf.i64("base"), Some(77));
    assert_eq!(odf.bool("VALID"), Some(true));
    assert_eq!(odf.raw("keyExtraTooLong"), None);
    assert_eq!(odf.source("absent"), None);
    assert!(!odf.has_rows("absent"));
    assert_eq!(odf.rows("absent").count(), 0);
}

#[test]
fn exact_default_still_uses_last_whole_assignment() {
    let odf = resolve(LookupPolicy::Exact);
    assert_eq!(odf.i64("key"), Some(33));
    assert_eq!(odf.source("key").unwrap().line, 6);
    assert!(!odf.has_rows("key"));
    assert_eq!(odf.raw("missing"), None);
    assert!(odf.lookup_changes().is_empty());
    let generic = sta_odf::resolve("x", &mut |_| Some("keyExtra=1\nkey=2\nkey=3\n".into())).unwrap();
    assert_eq!(generic.i64("key"), Some(3));
}

#[test]
fn audit_includes_unwritten_queries_and_sorts_without_hash_order() {
    let odf = resolve(LookupPolicy::PrefixFirst);
    let changes = odf.lookup_changes();
    let changed = changes.iter().find(|change| change.key == "key").unwrap();
    assert_eq!(changed.exact.as_deref(), Some("33"));
    assert_eq!(changed.selected, "11");
    assert_eq!(changed.source, *odf.source("key").unwrap());
    assert!(changes.iter().any(|change| change.key == "missing" && change.exact.is_none()));
    assert!(!changes.iter().any(|change| change.key == "keyextra")); // equal value
    assert!(changes.windows(2).all(|pair| pair[0].key < pair[1].key));
    assert_eq!(changes, resolve(LookupPolicy::PrefixFirst).lookup_changes());
}

#[test]
fn first_duplicate_and_include_order_survive_nested_resolution() {
    let odf = resolve_layered_with_policy("root", LookupPolicy::PrefixFirst, &mut |name, _| match name {
        "root.odf" => Some("#include \"left\"\n#include \"right\"\n".into()),
        "left.odf" => Some("#include \"deep\"\nkey=1\nkey=2\n".into()),
        "deep.odf" => Some("key=3\n".into()),
        "right.odf" => Some("key=4\n".into()),
        _ => None,
    }).unwrap();
    assert_eq!(odf.i64("key"), Some(1));
    assert_eq!(odf.source("key").unwrap().file.as_ref(), "left.odf");
    assert_eq!(odf.lookup_changes().iter().find(|c| c.key == "key").unwrap().exact.as_deref(), Some("4"));
}

#[test]
fn prefix_resolution_still_rejects_cycles_and_missing_includes() {
    for policy in [LookupPolicy::Exact, LookupPolicy::PrefixFirst] {
        assert!(matches!(resolve_layered_with_policy("a", policy, &mut |name, _| match name {
            "a.odf" => Some("#include \"b\"\nkey=1\n".into()),
            "b.odf" => Some("#include \"b\"\n".into()),
            _ => None,
        }), Err(sta_odf::OdfError::IncludeCycle(_))));
        assert!(matches!(resolve_layered_with_policy("a", policy, &mut |name, _| (name == "a.odf").then(|| "#include \"missing\"\n".into())), Err(sta_odf::OdfError::IncludeNotFound(_))));
    }
}
