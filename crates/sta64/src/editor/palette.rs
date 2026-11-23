//! ODF-driven map-editor placement palette.

use sta_content::{OdfIndex, Profile};
use sta_vfs::Vfs;

const ENTRY_COUNT: usize = 12;
const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    pub kind: EntryKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Branch(Vec<Entry>),
    Leaf { odf: String, neutral: bool },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Palette {
    pub(crate) root: Vec<Entry>,
}

impl Palette {
    pub(crate) fn load(vfs: &Vfs) -> Self {
        let (index, _) = OdfIndex::build(vfs);
        let a2_first = Profile::detect(vfs) == Profile::Sta2Classic;
        let fetch = |name: &str, version| index.load(vfs, name, version).map(String::into_bytes);
        let load = |name: &str| {
            if a2_first {
                sta_assets::a2fmt::odf::Odf::load_layered_first(name, fetch)
            } else {
                sta_assets::a2fmt::odf::Odf::load_layered(name, fetch)
            }
            .ok()
        };

        let Some(menu) = load("editmenu.odf") else {
            return Self::default();
        };
        let mut root = Vec::new();
        let mut active = Vec::new();
        for slot in 1..=ENTRY_COUNT {
            let Some(name) = menu.get_str(&format!("menuName{slot}")) else {
                continue;
            };
            let Some(file) = odf_file(name) else { continue };
            let Some((label, entries)) = load_page(&file, 0, &mut active, &load) else {
                continue;
            };
            root.push(Entry { label, kind: EntryKind::Branch(entries) });
        }
        Self { root }
    }
}

fn load_page(
    file: &str,
    depth: usize,
    active: &mut Vec<String>,
    load: &impl Fn(&str) -> Option<sta_assets::a2fmt::odf::Odf>,
) -> Option<(String, Vec<Entry>)> {
    let key = file.to_ascii_lowercase();
    if depth >= MAX_DEPTH || active.iter().any(|ancestor| ancestor == &key) {
        return None;
    }
    active.push(key);
    let result = (|| {
        let page = load(file)?;
        let stem = file.strip_suffix(".odf").unwrap_or(file);
        let title = page.get_str("menuTitle").map(str::trim).filter(|s| !s.is_empty())
            .unwrap_or(stem).to_owned();
        let build_items: Vec<String> = (1..=ENTRY_COUNT)
            .filter_map(|slot| page.get_str(&format!("buildItem{slot}")))
            .filter_map(odf_file)
            .collect();
        if !build_items.is_empty() {
            let mut entries = Vec::new();
            for child_file in build_items {
                if let Some((label, children)) = load_page(&child_file, depth + 1, active, load) {
                    entries.push(Entry { label, kind: EntryKind::Branch(children) });
                }
            }
            return Some((title, entries));
        }

        let neutral = page.get_bool("forceToNeutral").unwrap_or(false);
        let entries = (1..=ENTRY_COUNT)
            .filter_map(|slot| page.get_str(&format!("item{slot}")))
            .filter_map(odf_stem)
            .map(|odf| Entry { label: odf.clone(), kind: EntryKind::Leaf { odf, neutral } })
            .collect();
        Some((title, entries))
    })();
    active.pop();
    result
}

fn odf_file(name: &str) -> Option<String> {
    let name = name.trim().replace('\\', "/");
    let basename = name.rsplit('/').next()?.trim();
    let stem = if basename.len() >= 4 && basename[basename.len() - 4..].eq_ignore_ascii_case(".odf") {
        &basename[..basename.len() - 4]
    } else {
        basename
    };
    (!stem.is_empty()).then(|| format!("{stem}.odf"))
}

fn odf_stem(name: &str) -> Option<String> {
    let file = odf_file(name)?;
    let stem = file.strip_suffix(".odf")?;
    (!stem.is_empty()).then(|| stem.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Install(PathBuf);

    impl Install {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("sta64_palette_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, name: &str, text: &str) {
            let path = self.0.join("odf/other").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    impl Drop for Install {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn loads_nested_menu_and_stops_cycles() {
        let install = Install::new();
        install.file("editmenu.odf", "menuName1 = e_root.odf\n");
        install.file("e_root.odf", "menuTitle = Root\nbuildItem1 = e_group.odf\n");
        install.file("e_group.odf", "menuTitle = Group\nbuildItem1 = e_leaf.odf\nbuildItem2 = e_root.odf\n");
        install.file("e_leaf.odf", "menuTitle = Objects\nforceToNeutral = 1\nitem1 = Ships/Scout.ODF\nitem2 = cruiser\n");
        let mut vfs = Vfs::default();
        vfs.push_dir("test".into(), install.0.clone()).unwrap();

        let palette = Palette::load(&vfs);
        assert_eq!(palette.root.len(), 1);
        assert_eq!(palette.root[0].label, "Root");
        let EntryKind::Branch(group) = &palette.root[0].kind else { panic!("root is a branch") };
        assert_eq!(group.len(), 1, "the cycle back to e_root is skipped");
        assert_eq!(group[0].label, "Group");
        let EntryKind::Branch(objects) = &group[0].kind else { panic!("group is a branch") };
        assert_eq!(objects[0].label, "Objects");
        let EntryKind::Branch(leaves) = &objects[0].kind else { panic!("object page is a branch") };
        assert_eq!(leaves[0].kind, EntryKind::Leaf { odf: "Scout".into(), neutral: true });
        assert_eq!(leaves[1].kind, EntryKind::Leaf { odf: "cruiser".into(), neutral: true });
    }
}
