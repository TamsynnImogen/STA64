//! LUA-38: the engine's team strength variables on a tiny converted-shaped mission.
//!
//! The mission text wraps the converter's real prelude and engine (`tools/dsl_to_lua.py`), with the
//! variables bound as the converted Armada II missions bind them and rules shaped as the stock AI
//! plan switches are (a strength comparison against 2.0, a flag per plan, one load per switch).
use sta_core::{EntityId, Fixed, PlayerId, Vec3};
use sta_mission::api::{MissionConfig, PlayerNumbering};
use sta_mission::runtime::MissionRuntime;
use sta_sim::setup::Controller;
use sta_sim::view::{UnitFlags, UnitView, WorldView};

const CONVERTER: &str = include_str!("../../../tools/dsl_to_lua.py");

fn block(name: &str) -> &'static str {
    let start = CONVERTER.find(&format!("{name} = r\"\"\"")).expect("block") + name.len() + 7;
    let end = CONVERTER[start..].find("\"\"\"").expect("end of block");
    &CONVERTER[start..start + end]
}

fn mission() -> String {
    let head = r#"Mission_ = {
    name = 'tiny',
    vars = {}, state = {}, objects = {}, globals = {}, timers = {}, scripts = {},
    rules = {}, affected = {}, satisfied = {}, fired = {}, fireCount = {}, ran = {},
    calls = {}, missing = {}, failed = {}, notDecoded = {}, stepErrors = {},
    conditionErrors = {}, engineVars = {}, engineWritten = {}, engineReads = {}, stepCount = 0, callFailed = false,
    problems = {}, problemOrder = {}, blocked = {}, invalid = {}, unresolved = {},
}
"#;
    let body = r#"
for n = 1, 8 do BindVar('p_strength_team_' .. n, 'hot_float', EngineVar('strength_team_' .. n, 'hot_float')) end
BindVar('p_other_engine', 'hot_float', EngineVar('other_engine', 'hot_float'))
Mission.loads = {}
Mission.gated = 0
local function plan(name)
    return function()
        Mission.loads[#Mission.loads + 1] = name
        VarOp('p_AGGRESSIVE_AIP', 'set', name == 'aggressive' and 1 or 0)
        VarOp('p_DEFENSIVE_AIP', 'set', name == 'defensive' and 1 or 0)
        VarOp('p_EXPLORATION_AIP', 'set', name == 'exploration' and 1 or 0)
    end
end
M.aggressive, M.defensive, M.exploration = plan('aggressive'), plan('defensive'), plan('exploration')
function M.gated() Mission.gated = Mission.gated + 1 end
function M.ScriptExtendInterface(...)
    for _, f in ipairs({ 'AGGRESSIVE_AIP', 'DEFENSIVE_AIP', 'EXPLORATION_AIP' }) do BindVar('p_' .. f, 'hot_int', NewVar('hot_int', f)) end
    do return 51 end
end
function M.ScriptInit(...)
    for _, f in ipairs({ 'AGGRESSIVE_AIP', 'DEFENSIVE_AIP', 'EXPLORATION_AIP' }) do VarOp('p_' .. f, 'set', 0) end
    do return 0 end
end
local deps = { 'p_strength_team_1', 'p_strength_team_2', 'p_AGGRESSIVE_AIP', 'p_DEFENSIVE_AIP', 'p_EXPLORATION_AIP' }
Mission.rules[#Mission.rules + 1] = { name = 'SwitchToAggressiveAIPRule', action = 'aggressive', deps = deps,
    when = function(S, O) return (not T(S.p_AGGRESSIVE_AIP)) and 2.0 * S.p_strength_team_1 < S.p_strength_team_2 end }
Mission.rules[#Mission.rules + 1] = { name = 'SwitchToDefensiveAIPRule', action = 'defensive', deps = deps,
    when = function(S, O) return (not T(S.p_DEFENSIVE_AIP)) and S.p_strength_team_1 > 2.0 * S.p_strength_team_2 end }
Mission.rules[#Mission.rules + 1] = { name = 'SwitchToExplorationAIPRule', action = 'exploration', deps = deps,
    when = function(S, O) return (not T(S.p_EXPLORATION_AIP)) and S.p_strength_team_1 <= 2.0 * S.p_strength_team_2
        and 2.0 * S.p_strength_team_1 >= S.p_strength_team_2 end }
-- depends on the strengths only, gated by game time: it fires only when a recompute re-runs it
Mission.rules[#Mission.rules + 1] = { name = 'TimeGatedRule', action = 'gated', deps = { 'p_strength_team_1' },
    when = function(S, O) return Mission.time ~= nil and Mission.time > 60.0 end }
"#;
    format!("{head}{}\n{body}\n{}\n", block("PRELUDE"), block("ENGINE"))
}

fn unit(id: u32, owner: PlayerId, now: i32, max: i32, dying: bool) -> UnitView {
    UnitView {
        id: EntityId(id),
        def: sta_content::ContentKey::NONE,
        owner,
        pos: Vec3::default(),
        heading: (Fixed::ONE, Fixed::ZERO),
        speed: Fixed::ZERO,
        hull: [Fixed::from_int(now), Fixed::from_int(max)],
        shields: [[Fixed::ZERO; 2]; 4],
        flags: if dying { UnitFlags::DYING } else { UnitFlags::default() },
        progress: None,
        crew: None,
        energy: None,
        autonomy: None,
    }
}

fn seat(n: u8) -> PlayerId {
    PlayerId(n - 1)
}

fn view(tick: u64, units: Vec<UnitView>) -> WorldView {
    WorldView { tick, units, ..WorldView::default() }
}

fn runtime() -> MissionRuntime {
    let cfg = MissionConfig { player_numbering: PlayerNumbering::ArmadaTeams, ..MissionConfig::default() };
    let rt = MissionRuntime::from_source_with("missions/T.lua", &mission(), 1, cfg);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

fn value(rt: &MissionRuntime, n: u8) -> f64 {
    rt.eval::<f64>(&format!("return Mission_.vars.p_strength_team_{n}.value")).unwrap()
}

fn loads(rt: &MissionRuntime) -> String {
    rt.eval::<String>("return table.concat(Mission_.loads, ',')").unwrap()
}

/// Team 1 and team 2 craft of the given maximum hulls (and a neutral craft that never counts).
fn fleet(a: &[i32], b: &[i32]) -> Vec<UnitView> {
    let mut id = 0;
    let mut out = Vec::new();
    for (n, hulls) in [(1, a), (2, b)] {
        for &m in hulls {
            id += 1;
            out.push(unit(id, seat(n), m / 2, m, false));
        }
    }
    out.push(unit(900, sta_sim::world::NEUTRAL, 500, 5000, false));
    out
}

fn run(rt: &mut MissionRuntime, from: u64, to: u64, units: &dyn Fn(u64) -> Vec<UnitView>) {
    let mut out = Vec::new();
    for t in from..to {
        rt.think(&view(t, units(t)), &[], &mut out);
    }
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
}

#[test]
fn the_variables_read_zero_before_the_first_recompute_and_other_engine_floats_keep_their_start() {
    let rt = runtime();
    for n in 1..=8 {
        assert_eq!(value(&rt, n), 0.0, "team {n}");
    }
    assert_eq!(rt.eval::<f64>("return Mission_.vars.p_other_engine.value").unwrap(), -1.0);
}

#[test]
fn the_sum_is_the_max_hull_of_the_teams_craft_every_20_seconds_of_game_time() {
    let period = 20 * sta_core::TICK_HZ as u64;
    let mut rt = runtime();
    let mut units = fleet(&[100, 150], &[300]);
    // The first step already holds the sums: neutral craft and damage play no part.
    run(&mut rt, 0, 1, &|_| units.clone());
    assert_eq!((value(&rt, 1), value(&rt, 2), value(&rt, 3)), (250.0, 300.0, 0.0));
    // A new craft is not seen before the next recompute.
    units.push(unit(50, seat(1), 10, 50, false));
    units.sort_by_key(|u| u.id);
    run(&mut rt, 1, period, &|_| units.clone());
    assert_eq!(value(&rt, 1), 250.0);
    run(&mut rt, period, period + 1, &|_| units.clone());
    assert_eq!(value(&rt, 1), 300.0);
    // A dead craft (dying) leaves the sum at the next recompute, not before.
    units[0].flags = UnitFlags::DYING;
    run(&mut rt, period + 1, 2 * period, &|_| units.clone());
    assert_eq!(value(&rt, 1), 300.0);
    run(&mut rt, 2 * period, 2 * period + 1, &|_| units.clone());
    assert_eq!(value(&rt, 1), 200.0);
    // A team with no craft left reads 0 again.
    let none: Vec<UnitView> = Vec::new();
    run(&mut rt, 2 * period + 1, 3 * period + 1, &|t| if t < 3 * period { units.clone() } else { none.clone() });
    assert_eq!((value(&rt, 1), value(&rt, 2)), (0.0, 0.0));
}

#[test]
fn the_plan_switch_rules_load_once_and_hold_in_each_band() {
    // (team 1 hulls, team 2 hulls, the plan expected): 2 * 100 < 500; 500 > 2 * 100; evenly matched.
    for (a, b, plan) in [(&[100][..], &[500][..], "aggressive"), (&[500][..], &[100][..], "defensive"), (&[300][..], &[400][..], "exploration")] {
        let mut rt = runtime();
        let units = fleet(a, b);
        run(&mut rt, 0, 3000, &|_| units.clone());
        assert_eq!(loads(&rt), plan, "one load, then the flags hold across 100 s and five recomputes");
    }
}

#[test]
fn a_switch_follows_a_change_of_strength_once() {
    let mut rt = runtime();
    let weak = fleet(&[100], &[500]);
    let strong = fleet(&[600], &[100]);
    run(&mut rt, 0, 600, &|_| weak.clone());
    run(&mut rt, 600, 1800, &|_| strong.clone());
    assert_eq!(loads(&rt), "aggressive,defensive");
}

#[test]
fn every_recompute_re_runs_a_rule_that_depends_on_the_strengths_even_when_they_did_not_change() {
    let period = 20 * sta_core::TICK_HZ as u64;
    let mut rt = runtime();
    let units = fleet(&[100], &[100]);
    // The rule is gated by time > 60 s. Nothing but the recompute wakes it (the sums never move):
    // it is evaluated at 0, 20, 40 and 60 s (false: not later than 60) and fires at the one at 80 s.
    let count = |rt: &MissionRuntime| rt.eval::<i64>("return Mission_.fireCount['TimeGatedRule'] or 0").unwrap();
    run(&mut rt, 0, 4 * period, &|_| units.clone());
    assert_eq!(count(&rt), 0);
    run(&mut rt, 4 * period, 4 * period + 1, &|_| units.clone());
    assert_eq!(count(&rt), 1, "80 s is tick 2400, the fourth recompute after the first");
}
