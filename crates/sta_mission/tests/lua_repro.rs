//! Regression guard for #1300: a mission script with the same inputs gives the same results in
//! every Lua state. One authored script per hazard (`tests/lua_repro/*.lua`) folds what it sees
//! into `LOG`, one line per tick, over a fixed input stream. Each script runs in several fresh
//! states in this process, with the heap perturbed before each state is built, and in two child
//! processes (other addresses, other hash seeds); every log must equal the first.

use std::process::Command;

use sta_core::{EntityId, Fixed, PlayerId, Vec3};
use sta_mission::runtime::MissionRuntime;
use sta_sim::setup::Controller;
use sta_sim::view::{UnitFlags, UnitView, WorldView};

const STEPS: u64 = 60;
const PRELUDE: &str = include_str!("lua_repro/prelude.lua");

const HAZARDS: &[(&str, &str)] = &[
    ("H01_pairs_string_keys", include_str!("lua_repro/h01_pairs_string_keys.lua")),
    ("H02_pairs_table_keys", include_str!("lua_repro/h02_pairs_table_keys.lua")),
    ("H03_pairs_function_keys", include_str!("lua_repro/h03_pairs_function_keys.lua")),
    ("H04_pairs_value_keys_and_len", include_str!("lua_repro/h04_pairs_value_keys.lua")),
    ("H05_tostring_references", include_str!("lua_repro/h05_tostring.lua")),
    ("H05b_sort_by_tostring", include_str!("lua_repro/h05b_sort_by_tostring.lua")),
    ("H06_sort_ties", include_str!("lua_repro/h06_sort_ties.lua")),
    ("H07_math_random", include_str!("lua_repro/h07_random.lua")),
    ("H08_os_io", include_str!("lua_repro/h08_os_io.lua")),
    ("H09_weak_tables_gc", include_str!("lua_repro/h09_weak_gc.lua")),
    ("H09b_weak_values_no_address_keys", include_str!("lua_repro/h09b_weak_gc_clean.lua")),
    ("H10_coroutines", include_str!("lua_repro/h10_coroutines.lua")),
    ("H10b_thread_keys", include_str!("lua_repro/h10b_thread_keys.lua")),
    ("H11_float_format", include_str!("lua_repro/h11_float_format.lua")),
    ("H12_next_mutation_int", include_str!("lua_repro/h12_next_mutation_int.lua")),
    ("H12b_next_mutation_str", include_str!("lua_repro/h12b_next_mutation_str.lua")),
    ("H13_finder_order", include_str!("lua_repro/h13_finder_order.lua")),
    ("H13b_handle_keys", include_str!("lua_repro/h13b_handle_keys.lua")),
    ("H14_registry_pattern", include_str!("lua_repro/h14_registry_pattern.lua")),
];

/// Set in the child processes this test starts.
const CHILD_ENV: &str = "STA64_LUA_REPRO_CHILD";

fn unit(id: u32, tick: u64) -> UnitView {
    UnitView {
        id: EntityId(id),
        def: sta_content::ContentKey::NONE,
        owner: PlayerId((id % 3) as u8),
        pos: Vec3::default(),
        heading: (Fixed::ONE, Fixed::ZERO),
        speed: Fixed::ZERO,
        hull: [Fixed::from_int(100 - (tick % 7) as i32), Fixed::from_int(100)],
        shields: [[Fixed::ZERO; 2]; 4],
        flags: UnitFlags::default(),
        progress: None,
        crew: None,
        energy: None,
        autonomy: None,
    }
}

/// The recorded input stream: the same views every time; the unit order inside a view rotates
/// with the tick (as a sim's slot order may).
fn view(tick: u64) -> WorldView {
    let mut ids = [7u32, 3, 9, 1, 5, 12, 2, 8];
    ids.rotate_left((tick % 8) as usize);
    WorldView { tick, units: ids.iter().map(|i| unit(*i, tick)).collect(), ..WorldView::default() }
}

/// Heap allocations made before a Lua state is built, so its objects land at other addresses.
fn perturb(variant: u32) -> Vec<Box<[u8]>> {
    let mut keep: Vec<Box<[u8]>> = Vec::new();
    for i in 0..(variant as usize * 1500) {
        keep.push(vec![0u8; 16 + (i * 37) % 700].into_boxed_slice());
        if i % 3 == 0 {
            keep.swap_remove(i / 2);
        }
    }
    keep
}

fn run(src: &str) -> Vec<String> {
    let full = format!("{PRELUDE}\n{src}");
    let mut rt = MissionRuntime::from_source("missions/T.lua", &full, 12345);
    let mut commands = Vec::new();
    for t in 0..STEPS {
        rt.think(&view(t), &[], &mut commands);
    }
    let log: String = rt.eval("return table.concat(LOG, '\\n')").unwrap_or_else(|e| format!("EVALERR {e}"));
    let mut lines: Vec<String> = log.split('\n').map(str::to_owned).collect();
    lines.push(format!("COMMANDS {}", commands.len()));
    lines.extend(rt.errors().iter().map(|e| format!("ERROR {e}")));
    lines
}

fn fnv(lines: &[String]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in lines.iter().flat_map(|l| l.bytes().chain([b'\n'])) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn script(name: &str) -> &'static str {
    HAZARDS.iter().find(|(n, _)| *n == name).expect("hazard").1
}

#[test]
fn every_hazard_script_gives_the_same_log_in_every_state() {
    for (name, src) in HAZARDS {
        let first = run(src);
        assert_eq!(first.len() as u64, STEPS + 1, "{name}: one line per tick and the command count: {first:?}");
        assert!(!first.iter().any(|l| l.starts_with("ERROR") || l.starts_with("EVALERR")), "{name}: {first:?}");
        for variant in 1..4 {
            let _held = perturb(variant);
            let again = run(src);
            if let Some(i) = (0..first.len().max(again.len())).find(|i| first.get(*i) != again.get(*i)) {
                panic!("{name}: state {variant} differs at line {i}: {:?} vs {:?}", first.get(i), again.get(i));
            }
        }
    }
}

/// What the logs say, beyond being equal: the orders are the defined ones.
#[test]
fn hazard_logs_show_the_defined_orders() {
    let keys: Vec<String> = (1..=64).map(|i| format!("key{i}")).collect();
    let h01 = run(script("H01_pairs_string_keys"));
    assert_eq!(h01[0], keys.join(","));
    assert_eq!(h01[2], format!("{},extra1,extra2", keys.join(",")));
    let ids: Vec<String> = (1..=40).map(|i| i.to_string()).collect();
    assert_eq!(run(script("H02_pairs_table_keys"))[0], ids.join(","));
    assert_eq!(run(script("H03_pairs_function_keys"))[0], ids.join(","));
    assert_eq!(run(script("H10b_thread_keys"))[0], "1,2,3,4,5,6,7,8");
    // Finder results come sorted by id; the handle-keyed table keeps that insertion order.
    assert_eq!(run(script("H13b_handle_keys"))[0], "1,2,3,5,7,8,9,12");
    let h14 = run(script("H14_registry_pattern"));
    let names = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa"];
    assert_eq!(h14[2], names.map(|n| format!("{n}:3")).join(","));
    let h05 = run(script("H05_tostring_references"));
    assert!(h05.iter().all(|l| !l.contains("0x")), "no addresses: {:?}", &h05[..2]);
    assert!(h05[0].starts_with("table: #") && h05[0].contains(" function: #"), "{}", h05[0]);
    let h09 = run(script("H09_weak_tables_gc"));
    assert_eq!(h09[0], "300/300 weak=false:true gcguard=false fin=0", "weak tables refused, late __mode ignored");
    assert_eq!(h09[59], "18000/18000 weak=false:true gcguard=false fin=0");
    assert_eq!(run(script("H09b_weak_values_no_address_keys"))[59], "18000");
}

/// Child side of [`every_hazard_script_gives_the_same_log_in_other_processes`]: prints one
/// digest per script. Does nothing in a normal test run.
#[test]
fn child_prints_hazard_digests() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    for (name, src) in HAZARDS {
        println!("REPRO {name} {:016x}", fnv(&run(src)));
    }
}

#[test]
fn every_hazard_script_gives_the_same_log_in_other_processes() {
    if std::env::var_os(CHILD_ENV).is_some() {
        return;
    }
    let want: Vec<String> = HAZARDS.iter().map(|(name, src)| format!("REPRO {name} {:016x}", fnv(&run(src)))).collect();
    let exe = std::env::current_exe().expect("test binary path");
    for child in 0..2 {
        let out = Command::new(&exe)
            .args(["child_prints_hazard_digests", "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run the test binary as a child process");
        assert!(out.status.success(), "child {child}: {}", String::from_utf8_lossy(&out.stderr));
        let text = String::from_utf8_lossy(&out.stdout);
        // libtest may print the test name on the same line as the first digest.
        let got: Vec<&str> = text.lines().filter_map(|l| l.find("REPRO ").map(|i| &l[i..])).collect();
        assert_eq!(got, want, "child process {child} differs");
    }
}
