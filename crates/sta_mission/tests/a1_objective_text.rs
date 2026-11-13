//! LUA-16: Armada I objective texts. `ObjectivesDisplay_Set_Text_From_File`
//! names a flat file of the install's `objtxt.zfs` (mounted at the VFS root by
//! `sta_vfs`, CON-15); `A1` hands it to the shared objective loader that
//! Armada II and Roots missions use. The first test authors its own archive;
//! the second is install-gated (`STA64_TEST_A1_INSTALL`, needs `python3` and
//! `objdump` for the converter) and prints counts only, no game text.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sta_mission::campaign::*;
use sta_mission::runtime::MissionRuntime;
use sta_vfs::Vfs;

/// A ZFS v1 archive with key 0 (the `objtxt.zfs` layout), one block.
fn zfs(files: &[(&str, &[u8])]) -> Vec<u8> {
    const PER_BLOCK: usize = 100;
    const SLOT: usize = 16 + 20;
    let table = 28 + 4 + PER_BLOCK * SLOT;
    let mut out = b"ZFSF".to_vec();
    for v in [1u32, 16, PER_BLOCK as u32, files.len() as u32, 0, 28] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    let (mut at, mut data) = (table, Vec::new());
    for i in 0..PER_BLOCK {
        let mut rec = [0u8; SLOT];
        if let Some((name, bytes)) = files.get(i) {
            rec[..name.len()].copy_from_slice(name.as_bytes());
            for (j, v) in [at as u32, i as u32, bytes.len() as u32, 0, 0].iter().enumerate() {
                rec[16 + j * 4..20 + j * 4].copy_from_slice(&v.to_le_bytes());
            }
            data.extend_from_slice(bytes);
            at += bytes.len();
        }
        out.extend_from_slice(&rec);
    }
    out.extend(data);
    out
}

fn runtime(vfs: Arc<Vfs>) -> MissionRuntime {
    MissionRuntime::load("missions/T.lua", 1, move |p| {
        if p == "missions/T.lua" {
            return Some(b"-- empty controller\n".to_vec());
        }
        vfs.read(p).and_then(Result::ok)
    })
}

#[test]
fn objtxt_archive_feeds_the_objective_state_for_a1_calls() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("a1_objtxt_synthetic");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let text = b"   TEST MISSION\r\n\r\nOBJECTIVES:\r\n\r\n* Hold the line\r\n\r\n* Keep the ship alive\r\nHINT: be quick\r\n";
    std::fs::write(dir.join("objtxt.zfs"), zfs(&[("T1OB1.txt", text), ("t1ob2.txt", b"Only a story\r\n"), ("t1tm1.txt", b"1 MINUTE REMAINING")])).unwrap();
    let mut vfs = Vfs::default();
    vfs.push_dir("root".into(), dir).unwrap();
    let rt = runtime(Arc::new(vfs));

    // Name case follows the DLL's spelling, not the archive's.
    rt.eval::<()>("A1.ObjectivesDisplay_Set_Text_From_File('t1ob1.TXT')").unwrap();
    let goals: Vec<String> = rt.objectives().iter().map(|o| o.title.clone()).collect();
    assert_eq!(goals, ["Hold the line", "Keep the ship alive"]);
    let b = rt.objective_briefing().expect("briefing");
    assert_eq!(b.title, "TEST MISSION");
    assert_eq!(b.notes, "HINT: be quick");

    // A file without goals still sets the text; a later file replaces the state.
    rt.eval::<()>("A1.ObjectivesDisplay_Set_Text_From_File('T1OB2.txt')").unwrap();
    assert!(rt.objectives().is_empty());
    assert_eq!(rt.objective_briefing().unwrap().title, "Only a story");

    // A name the archive lacks leaves the state alone and is counted.
    rt.eval::<()>("A1.ObjectivesDisplay_Set_Text_From_File('nope.txt')").unwrap();
    assert_eq!(rt.objective_briefing().unwrap().title, "Only a story");
    assert_eq!(rt.unsupported().iter().filter(|(k, _)| k == "Objectives.load").count(), 1);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

fn convert(install: &str, tag: &str) -> PathBuf {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join(tag);
    let _ = std::fs::remove_dir_all(&out);
    let run = std::process::Command::new("python3")
        .arg("-I")
        .arg(repo.join("tools/a1_to_lua.py"))
        .args(["--install", install, "--out"])
        .arg(&out)
        .output()
        .expect("run python3 tools/a1_to_lua.py");
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    out
}

/// The text names the converted Lua passes to the A1 call.
fn referenced(missions: &Path) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for e in std::fs::read_dir(missions.join("missions")).unwrap() {
        let src = std::fs::read_to_string(e.unwrap().path()).unwrap();
        for line in src.lines() {
            let Some(rest) = line.trim().strip_prefix("A('ObjectivesDisplay_Set_Text_From_File', '") else { continue };
            names.insert(rest.split('\'').next().unwrap().to_owned());
        }
    }
    names
}

#[test]
fn every_objective_text_a_converted_a1_mission_issues_resolves() {
    let Ok(install) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("skipped: set STA64_TEST_A1_INSTALL to an Armada I root");
        return;
    };
    let out = convert(&install, "a1_objective_text");
    let mut vfs = Vfs::mount_install(Path::new(&install), None).expect("mount install");
    vfs.push_dir("converted".into(), out.clone()).expect("mount the converted missions");
    let vfs = Arc::new(vfs);

    // Static: every file name any converted mission passes loads to a
    // non-empty text (a goal list, or a story for the goal-less files).
    let names = referenced(&out);
    assert!(names.len() >= 60, "{}", names.len());
    let (mut with_goals, mut story_only, mut goals) = (0, 0, 0);
    for name in &names {
        let rt = runtime(vfs.clone());
        rt.eval::<()>(&format!("A1.ObjectivesDisplay_Set_Text_From_File('{name}')")).unwrap();
        assert!(rt.unsupported().is_empty(), "{name}: not found in the install");
        let b = rt.objective_briefing().unwrap_or_else(|| panic!("{name}: no text"));
        assert!(!b.title.is_empty(), "{name}: empty title");
        goals += rt.objectives().len();
        if rt.objectives().is_empty() {
            assert!(!b.text.is_empty(), "{name}: no goals and no text");
            story_only += 1;
        } else {
            with_goals += 1;
        }
    }
    eprintln!("{} referenced texts: {with_goals} with goals ({goals} goals), {story_only} text only", names.len());

    // Dynamic: what missions issue in their first ticks reaches the state.
    const TICKS: u64 = 3000;
    let mut book = CampaignBook::load_vfs(&vfs);
    book.give_all_missions = true;
    let host = VfsHost::load(vfs.clone()).expect("installed content loads");
    let mut runner = CampaignRunner::new(book.clone());
    let (mut issuing, mut resolved, mut unresolved) = (0, BTreeSet::new(), 0);
    for c in &book.campaigns {
        for m in c.missions.iter().filter(|m| matches!(m.script, MissionScript::Lua { .. })) {
            let a = runner.start(&host, &c.id, m.index, 1).expect("mission starts");
            let mut seen = BTreeSet::new();
            for _ in 0..TICKS {
                let ended = a.tick();
                if let Some(b) = a.runtime().objective_briefing()
                    && seen.insert(b.file.clone())
                {
                    assert!(!b.title.is_empty(), "{}: {}", m.bzn, b.file);
                }
                if ended.is_some() {
                    break;
                }
            }
            let missing: u32 = a.runtime().unsupported().iter().filter(|(k, _)| k == "Objectives.load").map(|(_, n)| *n).sum();
            unresolved += missing;
            issuing += usize::from(!seen.is_empty() || missing > 0);
            resolved.extend(seen);
            runner.abandon();
        }
    }
    eprintln!("first {TICKS} ticks: {issuing} missions issue objectives, {} distinct texts resolved, {unresolved} unresolved", resolved.len());
    assert_eq!(unresolved, 0);
}
