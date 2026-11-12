//! The D10 rules check between two in-process peers (CON-35): each peer
//! mounts its own install, loads its `ContentDb`, and the host compares the
//! joiner's rules the way the handshake does (`Hello` digest, the joiner's
//! entries as `Rules` frames through a `RulesAssembler`, `rules_refusal`).
//! No transport: NET-02/03 carry the same messages.
//!
//! Synthetic layers run in CI. With `STA64_TEST_INSTALL` set to the Fleet
//! Operations Roots install, STA2 Classic and STA2 Classic Enhanced refuse
//! each other and name the differing key, and every installed mod's rule
//! list fits the wire bounds.

use std::path::{Path, PathBuf};

use sta_content::{ContentDb, OdfIndex, Profile};
use sta_net::proto::{
    Message, Refusal, RuleDiff, RulesAssembler, RulesIdentity, decode, encode, rules_parts,
    rules_refusal,
};
use sta_vfs::Vfs;

const GLB: &[u8] = include_bytes!("../../../fixtures/models/fx_cruiser.glb");
const SHIP: &str =
    "classLabel = \"craft\"\nunitName = \"Ship\"\nbasename = \"fx_ship\"\nScaleSOD = 2.0\nmaxHealth = 500\n";
/// The eight lines STA2 Classic Enhanced Gameplay changes, Classic's values.
const CLASSIC_CFG: &str = "float HARD_AI_COST_HANDICAP = 0.6;\nfloat HARD_AI_SPEED_HANDICAP = 0.7;\n\
    int cfgFREIGHTERS_PER_NEBULA = 1;\nfloat SHIELD_DAMAGE_PROTECTION = 0.8;\nfloat REPAIR_SHIP_DISTANCE = 180;\n";
/// Enhanced changes one of them.
const ENHANCED_CFG: &str = "float HARD_AI_COST_HANDICAP = 0.6;\nfloat HARD_AI_SPEED_HANDICAP = 0.7;\n\
    int cfgFREIGHTERS_PER_NEBULA = 1;\nfloat SHIELD_DAMAGE_PROTECTION = 0.9; // JanB: increased from 0.8\n\
    float REPAIR_SHIP_DISTANCE = 180;\n";

fn s(out: &mut Vec<u8>, t: &str) {
    out.extend_from_slice(&(t.len() as u16).to_le_bytes());
    out.extend_from_slice(t.as_bytes());
}
fn f(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn h(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// A minimal SOD 1.93: a root node and a one-triangle mesh `size` wide
/// textured with `texture` (render data only).
fn sod(size: f32, texture: &str) -> Vec<u8> {
    let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
    let mut d = b"Storm3D_SW".to_vec();
    f(&mut d, 1.93);
    h(&mut d, 1);
    s(&mut d, "team");
    for _ in 0..10 {
        f(&mut d, 0.5);
    }
    d.push(2);
    d.push(1);
    h(&mut d, 2);
    h(&mut d, 0);
    s(&mut d, "root");
    s(&mut d, "");
    identity.iter().for_each(|v| f(&mut d, *v));
    h(&mut d, 1);
    s(&mut d, "hull");
    s(&mut d, "ROOT");
    identity.iter().for_each(|v| f(&mut d, *v));
    s(&mut d, "opaque");
    d.extend_from_slice(&4u32.to_le_bytes());
    d.extend_from_slice(&1u32.to_le_bytes());
    s(&mut d, texture);
    d.extend_from_slice(&0u32.to_le_bytes());
    s(&mut d, "");
    h(&mut d, 0);
    h(&mut d, 3);
    h(&mut d, 2);
    h(&mut d, 1);
    for p in [[0.0, 0.0, 0.0], [size, 0.0, 0.0], [0.0, size, 0.0]] {
        p.iter().for_each(|c| f(&mut d, *c));
    }
    for t in [[0.0, 0.0], [1.0, 1.0]] {
        t.iter().for_each(|c| f(&mut d, *c));
    }
    h(&mut d, 1);
    s(&mut d, "team");
    for (p, t) in [(0, 0), (1, 1), (2, 0)] {
        h(&mut d, p);
        h(&mut d, t);
    }
    d.push(1);
    h(&mut d, 0);
    h(&mut d, 0);
    h(&mut d, 0);
    d
}

/// A temporary layer directory, removed on drop.
struct Layer(PathBuf);

impl Layer {
    fn new(tag: &str, files: &[(&str, &[u8])]) -> Layer {
        let dir = std::env::temp_dir().join(format!("sta_rules_handshake_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (rel, bytes) in files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        Layer(dir)
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The synthetic base install: `RTS_CFG.h`, one ship and its SOD.
fn data(tag: &str) -> Layer {
    let model = sod(10.0, "fhull");
    Layer::new(
        tag,
        &[
            ("RTS_CFG.h", CLASSIC_CFG.as_bytes()),
            ("odf/ship.odf", SHIP.as_bytes()),
            ("sod/fx_ship.sod", &model),
        ],
    )
}

/// A peer's content over `layers` (top first), each named as a pack.
fn peer(layers: &[(&str, &Layer)]) -> ContentDb {
    let mut vfs = Vfs::default();
    for (name, l) in layers {
        vfs.push_dir((*name).to_owned(), l.0.clone()).unwrap();
    }
    let (index, _) = OdfIndex::build(&vfs);
    ContentDb::load_as(&vfs, Profile::Sta2Classic, &index).unwrap().0
}

/// The host's verdict on `joiner`: its `Hello` digest, then (on a mismatch)
/// its entries sent as `Rules` frames and reassembled.
fn handshake(host: &ContentDb, joiner: &ContentDb) -> Option<Refusal> {
    let hello = RulesIdentity::digest(joiner.digest());
    let mut assembler = RulesAssembler::default();
    let mut got = None;
    for part in rules_parts(&RulesIdentity::entries(joiner.digest())).unwrap() {
        let frame = encode(&Message::Rules(part)).unwrap();
        let Message::Rules(part) = decode(&frame).unwrap() else { unreachable!() };
        got = assembler.push(part).unwrap();
    }
    rules_refusal(host.digest(), hello, &got.expect("every part arrived")).unwrap()
}

#[test]
fn classic_and_enhanced_refuse_and_name_the_key() {
    let base = data("cfg_base");
    let enhanced = Layer::new("cfg_enh", &[("RTS_CFG.h", ENHANCED_CFG.as_bytes())]);
    let classic = peer(&[("Data", &base)]);
    let other = peer(&[("Enhanced", &enhanced), ("Data", &base)]);
    let Some(Refusal::RulesDiffer(diff)) = handshake(&classic, &other) else {
        panic!("Classic and Enhanced must refuse with a named key")
    };
    assert_eq!(
        diff,
        RuleDiff {
            key: "RTS_CFG.h:SHIELD_DAMAGE_PROTECTION".into(),
            host: Some(sta_content::digest::fixed_text(sta_content::Fixed::from_f64(0.8f32 as f64))),
            joiner: Some(sta_content::digest::fixed_text(sta_content::Fixed::from_f64(0.9f32 as f64))),
        }
    );
    // Both sides name it the same way.
    assert_eq!(handshake(&other, &classic).map(|r| matches!(r, Refusal::RulesDiffer(d) if d.key == diff.key)), Some(true));
}

#[test]
fn art_only_differences_are_accepted() {
    let base = data("art_base");
    let host = peer(&[("Data", &base)]);
    let retextured = sod(10.0, "fhull_hd");
    let cases = [
        // A texture pack.
        Layer::new("art_tex", &[("textures/rgb/fhull.tga", b"not really a tga")]),
        // A render recipe folder.
        Layer::new("art_recipe", &[("recipes/nebula/mutara.ini", b"[layer]\nalpha = 0.5\n")]),
        // A render-only model beside the profile's SOD.
        Layer::new("art_glb", &[("sod/fx_ship.glb", GLB), ("sod/fx_ship.gltf", b"{}")]),
        // A geometry-identical re-export of the SOD (another texture name).
        Layer::new("art_reexport", &[("sod/fx_ship.sod", &retextured)]),
    ];
    for (i, layer) in cases.iter().enumerate() {
        let joiner = peer(&[("Some Pack", layer), ("Data", &base)]);
        assert_eq!(joiner.packs(), ["Some Pack", "Data"]);
        assert_eq!(handshake(&host, &joiner), None, "case {i}");
        assert_eq!(joiner.hash(), host.hash(), "case {i}");
    }
}

#[test]
fn a_unit_whose_geometry_differs_is_refused_with_the_unit_and_the_value() {
    let base = data("geo_base");
    let host = peer(&[("Data", &base)]);
    let bigger = sod(20.0, "fhull");
    let layer = Layer::new("geo_big", &[("sod/fx_ship.sod", &bigger)]);
    let joiner = peer(&[("Bigger Ships", &layer), ("Data", &base)]);
    let Some(Refusal::RulesDiffer(diff)) = handshake(&host, &joiner) else {
        panic!("a geometry change must refuse")
    };
    assert_eq!(diff.key, "geometry:ship");
    let radius = |db: &ContentDb| db.unit(db.key("ship").unwrap()).unwrap().radius;
    assert!(radius(&joiner) > radius(&host));
    assert_eq!(diff.host, Some(format!("radius={}", sta_content::digest::fixed_text(radius(&host)))));
    assert_eq!(diff.joiner, Some(format!("radius={}", sta_content::digest::fixed_text(radius(&joiner)))));
}

#[test]
fn a_changed_unit_is_refused_by_its_definition_key() {
    let base = data("unit_base");
    let host = peer(&[("Data", &base)]);
    let tougher = SHIP.replace("maxHealth = 500", "maxHealth = 600");
    let layer = Layer::new("unit_hull", &[("odf/ship.odf", tougher.as_bytes())]);
    let joiner = peer(&[("Tough Ships", &layer), ("Data", &base)]);
    let Some(Refusal::RulesDiffer(diff)) = handshake(&host, &joiner) else {
        panic!("a unit change must refuse")
    };
    // The key names the definition. Its value is longer than the wire's
    // bound, so both sides see it hashed (`#<len>:<fnv>`)...
    assert_eq!(diff.key, "unit:ship");
    assert!(diff.host.as_deref().is_some_and(|v| v.starts_with('#')), "{diff:?}");
    assert_ne!(diff.host, diff.joiner);
    // ... while a peer holding both lists names the field.
    let local = host.digest().first_difference(joiner.digest()).unwrap();
    assert_eq!(local.field().map(|f| f.0), Some("hull".to_owned()));
}

#[test]
fn the_digest_is_the_same_on_every_load() {
    let base = data("det_base");
    let a = peer(&[("Data", &base)]);
    let b = peer(&[("Renamed", &base)]);
    assert_eq!(a.digest(), b.digest());
    assert_eq!(handshake(&a, &b), None);
}

// ------------------------------------------------------------ installs

fn install() -> Option<PathBuf> {
    std::env::var_os("STA64_TEST_INSTALL").map(PathBuf::from)
}

fn installed(root: &Path, active: Option<&str>) -> Option<ContentDb> {
    let vfs = Vfs::mount_install(root, active).ok()?;
    let (index, _) = OdfIndex::build(&vfs);
    Some(ContentDb::load(&vfs, &index).unwrap().0)
}

#[test]
fn installed_classic_and_enhanced_refuse_and_name_the_differing_key() {
    let Some(root) = install() else { return };
    let (Some(classic), Some(enhanced)) = (
        installed(&root, Some("STA2 Classic Mod")),
        installed(&root, Some("STA2 Classic Mod Enhanced Gameplay")),
    ) else {
        return; // not a Roots install with both mods
    };
    let refusal = handshake(&classic, &enhanced);
    eprintln!("Classic host, Enhanced joiner: {refusal:?}");
    let Some(Refusal::RulesDiffer(diff)) = refusal else { panic!("{refusal:?}") };
    // The header keys sort first; Enhanced adds BASE_CREW_PER_SEC.
    assert_eq!(diff.key, "RTS_CFG.h:BASE_CREW_PER_SEC");
    assert_eq!((diff.host.as_deref(), diff.joiner.as_deref()), (Some("unset"), Some("30")));
    let all = classic.digest().differences(enhanced.digest());
    let cfg: Vec<&str> = all.iter().map(|d| d.key.as_str()).filter(|k| k.starts_with("RTS_CFG.h:")).collect();
    assert_eq!(cfg, sta_content::CfgRules::KEYS.map(|k| format!("RTS_CFG.h:{k}")));
    eprintln!("{} differing keys in all; the first ten:", all.len());
    for d in all.iter().take(10) {
        eprintln!("  {d}");
    }
    eprintln!("packs: {:?} / {:?}; lineage {} / {}", classic.packs(), enhanced.packs(), classic.lineage(), enhanced.lineage());
}

#[test]
fn every_installed_mod_fits_the_wire_and_digests_alike_twice() {
    let Some(root) = install() else { return };
    let data = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case("data"))
        .map(|e| e.path());
    let mut mods: Vec<Option<String>> = vec![None];
    if let Some(data) = data
        && let Ok((list, _)) = sta_vfs::mods::list_mods(&data)
    {
        mods.extend(list.into_iter().map(|m| Some(m.folder)));
    }
    for active in mods {
        let Some(db) = installed(&root, active.as_deref()) else { continue };
        let again = installed(&root, active.as_deref()).unwrap();
        assert_eq!(db.digest(), again.digest(), "{active:?}");
        let entries = RulesIdentity::entries(db.digest());
        let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
        for e in &entries {
            *kinds.entry(e.key.split(':').next().unwrap_or("")).or_default() += 1;
        }
        eprintln!("{active:?}: {kinds:?}");
        let parts = rules_parts(&entries).unwrap_or_else(|e| panic!("{active:?}: {e:?}"));
        eprintln!("{active:?}: {} entries, {} parts, hash {:#018x}", entries.len(), parts.len(), db.hash());
        assert_eq!(handshake(&db, &again), None);
    }
}
