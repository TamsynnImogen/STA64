use std::sync::Arc;
use sta_sim::{Sim,WorldSource,PlayerId,TeamId};
use sta_sim::setup::{GameSetup,GameOptions,PlayerSetup,Control};
fn main() {
    assert_eq!(sta_sim::snapshot::FORMAT_VERSION,12);
    assert_eq!(sta_sim::SEAM_REV,3);
    let db=Arc::new(sta_content::fake::db());
    let (races,warnings)=sta_content::Races::load_with(&mut |name,_| match name {
        "races.odf"=>Some("race0=fx".into()),
        "fx.odf"=>Some("name=fx\ninstantActionSlot=0\nstandardUnits1=fx_cruiser\nnormalCrew=1200".into()),
        _=>None,
    });
    assert!(warnings.is_empty(),"{warnings:?}");
    let mut map=sta_content::MapDef::empty();map.starts=sta_content::fake::two_bases().starts;
    let setup=GameSetup{seed:83,map:Arc::new(map),content:db.clone(),players:vec![PlayerSetup{id:PlayerId(0),team:TeamId(0),race:"fx".into(),control:Control::Human}],options:GameOptions::default()};
    let (mut sim,warnings)=Sim::from_setup_with_starts(&setup,&races);
    assert!(warnings.is_empty(),"{warnings:?}");
    for _ in 0..5 {sim.step(&[]);}
    let bytes=sim.snapshot();std::fs::write(std::env::args().nth(1).unwrap(),&bytes).unwrap();
    println!("version12 seam3 content={:016x} tick5 digest={:016x} bytes={} crew={:?}",db.hash(),sim.digest(),bytes.len(),sim.view().player(PlayerId(0)).unwrap().crew);
    for _ in 0..30 {sim.step(&[]);}
    println!("tick35 digest={:016x}",sim.digest());
}
