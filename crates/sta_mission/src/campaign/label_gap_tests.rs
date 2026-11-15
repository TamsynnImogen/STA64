//! `CampaignRunner::start` hands the host's label gaps to the mission before its chunk loads (LUA-25).

use std::collections::BTreeSet;

use super::*;
use crate::map_labels::{LabelGap, LabelKind};

const MSHELL: &str = "// drills\r\nsyn_tut1.bzn\r\n-\r\n// first fleet\r\nsyn_fed01.bzn // opener\r\n-\r\n-\r\n-\r\n";
const LABELS: &str = "single_player {\n\t+ federation_incomplete\t{Finish the first fleet}\n\t+ klingon_incomplete {}\n\t+ tutorials {Drills}\n\t+ federation_campaign {First Fleet}\n\t+ klingon_campaign {Second Fleet}\n}\nmission_select {\n\t+ syn_fed01.bzn\t{First Light}\n}\n";

struct GapHost(Arc<MemSource>, Vec<LabelGap>);

impl MissionHost for GapHost {
    fn game_setup(&self, _c: &Campaign, _m: &CampaignMission, seed: u64) -> Result<GameSetup, String> {
        let player = |id: u8, control| PlayerSetup { id: PlayerId(id), team: TeamId(id), race: "fx".into(), control };
        Ok(GameSetup {
            seed,
            map: Arc::new(MapDef::empty()),
            content: Arc::new(sta_content::fake::db()),
            players: vec![player(0, Control::Human), player(1, Control::Idle)],
            options: GameOptions::default(),
        })
    }

    fn script(&self, _c: &Campaign, m: &CampaignMission) -> Result<(String, ScriptReader), String> {
        let src = self.0.clone();
        resolved_script(m, Arc::new(move |p| src.read(p)))
    }

    fn map_label_gaps(&self, _c: &Campaign, _m: &CampaignMission, _carried: &BTreeSet<String>) -> Vec<LabelGap> {
        self.1.clone()
    }
}

fn run(gaps: Vec<LabelGap>) -> Vec<(String, u32)> {
    let src = Arc::new(
        MemSource::new()
            .with("mshell.set", MSHELL)
            .with("label.map", LABELS)
            .with("ART_CFG.h", "int GIVE_ALL_MISSIONS = 0; // off\r\n")
            .with("bzn/syn_tut1.bzn", "not parsed")
            .with("bzn/syn_fed01.bzn", "not parsed")
            .with("missions/syn_fed01.lua", "H = A2.GetHandle('Cloud1')\nM = A2.GetHandle('Sentinel')\n"),
    );
    let book = CampaignBook::load(&*src, ShellLayout::Armada2);
    let mut r = CampaignRunner::new(book);
    let a = r.start(&GapHost(src, gaps), "federation", 0, 1).unwrap();
    a.run(5);
    a.runtime().unsupported().into_iter().filter(|(k, _)| k.contains("GetHandle")).collect()
}

#[test]
fn the_hosts_gaps_reach_a_get_handle_that_runs_at_chunk_load() {
    let got = run(vec![LabelGap { label: "Cloud1".into(), odf: "fx_cloud".into(), kind: LabelKind::Region }]);
    assert_eq!(got.len(), 1, "only the map label is a gap, not the sentinel: {got:?}");
    assert!(got[0].0.contains("label 'Cloud1'"), "{got:?}");
}

#[test]
fn a_host_with_no_gaps_records_none() {
    assert!(run(Vec::new()).is_empty());
}
