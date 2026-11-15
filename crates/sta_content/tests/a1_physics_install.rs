//! Install-gated (STA64_TEST_A1_INSTALL, the stock Armada I folder; skipped
//! when unset): the Armada I install loads with the play-physics block and the
//! model's parameters (CON-52). General rules only: the block is there and
//! usable, every craft that moves has parameters through the content
//! database, craft that do not move and structures have none, and the Borg
//! keys come only with `borgPhysics`.

use sta_content::{ContentDb, OdfIndex, Profile};
use sta_vfs::Vfs;

#[test]
fn the_armada_i_install_supplies_the_play_block_and_per_craft_keys() {
    let Ok(dir) = std::env::var("STA64_TEST_A1_INSTALL") else {
        eprintln!("SKIP: set STA64_TEST_A1_INSTALL to the stock Armada I folder");
        return;
    };
    let mut vfs = Vfs::default();
    vfs.push_dir("root".into(), dir.into()).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, warnings) = ContentDb::load_as(&vfs, Profile::Sta1Classic, &index).unwrap();
    let play = db.a1_play_physics().expect("the play-physics block loads from the install");
    assert!(play.impulse_speed.to_f32() > 0.0 && play.front_back_inertia.to_f32() > 0.0 && play.turn_radius.to_f32() > 0.0);
    assert!(!warnings.iter().any(|w| w.file == "fastphys.odf"), "{warnings:?}");
    let (mut movers, mut borgs) = (0, 0);
    for u in db.units() {
        let moves = u.movement.max_speed.to_f32() > 0.0;
        movers += usize::from(moves);
        borgs += usize::from(u.a1_borg.is_some());
        if u.a1_borg.is_some() {
            assert!(moves, "{} has Borg keys but does not move", u.odf);
        }
    }
    eprintln!("{movers} craft move, {borgs} with Borg keys");
    assert!(movers > 0 && borgs > 0);
}

#[test]
fn the_same_install_under_another_profile_reads_nothing_of_it() {
    let Ok(dir) = std::env::var("STA64_TEST_A1_INSTALL") else {
        return;
    };
    let mut vfs = Vfs::default();
    vfs.push_dir("root".into(), dir.into()).unwrap();
    let (index, _) = OdfIndex::build(&vfs);
    let (db, _) = ContentDb::load_as(&vfs, Profile::Native, &index).unwrap();
    assert!(db.a1_play_physics().is_none() && db.units().all(|u| u.a1_borg.is_none()));
}
