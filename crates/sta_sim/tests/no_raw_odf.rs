//! D9: the simulation reads no raw ODF. `UnitDef::raw` and `WeaponDef::raw`
//! stay public for loaders and tools, and a method on a field's type needs no
//! dependency on `sta_odf`, so the rule is checked on the source: no file
//! under `src/` touches a definition's `.raw`. What the sim needs is a typed
//! field filled by the content loader (and part of the content digest).

use std::path::Path;

fn scan(dir: &Path, hits: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan(&path, hits);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                let mut rest = code;
                while let Some(i) = rest.find(".raw") {
                    let after = rest[i + 4..].chars().next();
                    if !after.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        hits.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                    }
                    rest = &rest[i + 4..];
                }
            }
        }
    }
}

#[test]
fn the_sim_reads_no_raw_odf() {
    let mut hits = Vec::new();
    scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut hits);
    assert!(hits.is_empty(), "raw ODF reads in sta_sim (type them in sta_content):\n{}", hits.join("\n"));
}
