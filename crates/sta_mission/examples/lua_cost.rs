//! Cost of the mission Lua runtime on table-heavy authored work (issue #1300): time per
//! workload, memory per table, and time per mission tick. A measuring tool, not a test: it reads
//! the wall clock. `cargo run -p sta_mission --example lua_cost` (each workload runs `REPEAT`
//! times in a fresh state; the median is printed).

use std::time::Instant;

use sta_core::{EntityId, Fixed, PlayerId, Vec3};
use sta_mission::runtime::MissionRuntime;
use sta_mission::sandbox;
use sta_sim::setup::Controller;
use sta_sim::view::{UnitFlags, UnitView, WorldView};

const REPEAT: usize = 7;

/// Each chunk stays under the per-entry instruction budget.
const WORKLOADS: &[(&str, &str)] = &[
    (
        "insert: 2000 tables x (40 string + 40 integer + 10 table keys)",
        "local all = {} for t = 1, 2000 do local x = {} for i = 1, 40 do x['k' .. i] = i end \
         for i = 1, 40 do x[i] = i end for i = 1, 10 do x[{}] = i end all[t] = x end",
    ),
    (
        "lookup: 2,000,000 string-key reads on a 64-key table",
        "local x = {} for i = 1, 64 do x['k' .. i] = i end local s = 0 \
         local a, b, c, d = 'k1', 'k17', 'k33', 'k64' \
         for i = 1, 500000 do s = s + x[a] + x[b] + x[c] + x[d] end",
    ),
    (
        "traverse: pairs over 10,000 string keys, 60 times",
        "local x = {} for i = 1, 10000 do x['k' .. i] = i end local s = 0 \
         for r = 1, 60 do for k, v in pairs(x) do s = s + v end end",
    ),
    (
        "churn: 200,000 insert/delete pairs of distinct string keys",
        "local x = {} local keys = {} for i = 1, 2000 do keys[i] = 'q' .. i end \
         for i = 1, 200000 do local k = keys[i % 2000 + 1] x[k] = i x[keys[(i + 1000) % 2000 + 1]] = nil end",
    ),
    (
        "sort: 20,000 numbers, default comparator",
        "local a = {} local v = 12345 for i = 1, 20000 do v = (v * 1103515245 + 12345) % 2147483648 a[i] = v end \
         table.sort(a)",
    ),
    (
        "sort: 5,000 records, Lua comparator on a key with ties",
        "local a = {} for i = 1, 5000 do a[i] = { id = i, key = (i * 7919) % 97 } end \
         table.sort(a, function(x, y) return x.key < y.key end)",
    ),
];

/// One mission tick of an authored script: a name-keyed handler registry walked with `pairs`,
/// a few fresh tables, a sort and a `tostring` per tick.
const MISSION: &str = r#"
local handlers, state = {}, { seen = {} }
for _, name in ipairs({ "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta" }) do
  handlers[name] = function(n) state[name] = (state[name] or 0) + n end
end
local n = 0
MMM.register(function()
  n = n + 1
  for name, h in pairs(handlers) do h(n) end
  local list = EntityFinder():find()
  local byid = {}
  for _, e in ipairs(list) do byid[e] = e.id end
  local ids = {}
  for _, id in pairs(byid) do ids[#ids + 1] = { id = id, w = id % 3 } end
  table.sort(ids, function(a, b) return a.w < b.w end)
  state.seen[#state.seen % 64 + 1] = tostring(ids)
end)
"#;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn unit(id: u32) -> UnitView {
    UnitView {
        id: EntityId(id),
        def: sta_content::ContentKey::NONE,
        owner: PlayerId((id % 3) as u8),
        pos: Vec3::default(),
        heading: (Fixed::ONE, Fixed::ZERO),
        speed: Fixed::ZERO,
        hull: [Fixed::from_int(100), Fixed::from_int(100)],
        shields: [[Fixed::ZERO; 2]; 4],
        flags: UnitFlags::default(),
        progress: None,
        crew: None,
        energy: None,
        autonomy: None,
    }
}

fn main() {
    for (name, chunk) in WORKLOADS {
        let times: Vec<f64> = (0..REPEAT)
            .map(|_| {
                let lua = sandbox::new_lua();
                sandbox::refill(&lua);
                let start = Instant::now();
                lua.load(*chunk).exec().unwrap_or_else(|e| panic!("{name}: {e}"));
                start.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        println!("{:>9.2} ms  {name}", median(times));
    }

    for (name, keys) in [("empty table", 0), ("8 string keys", 8), ("64 string keys", 64), ("8 integer keys (array)", -8)] {
        let lua = sandbox::new_lua();
        let fill = match keys {
            0 => String::new(),
            k if k > 0 => format!("for i = 1, {k} do x['k' .. i] = i end"),
            k => format!("for i = 1, {} do x[i] = i end", -k),
        };
        let before = lua.used_memory();
        lua.load(format!("KEEP = {{}} for t = 1, 10000 do local x = {{}} {fill} KEEP[t] = x end")).exec().expect("fill");
        let after = lua.used_memory();
        // The KEEP array itself: 10000 slots of 16 bytes (array part grown by doubling, at most 2x).
        println!("{:>9.1} B   per table, {name} (incl. KEEP slot)", (after - before) as f64 / 10000.0);
    }

    let units: Vec<UnitView> = [7u32, 3, 9, 1, 5, 12, 2, 8].iter().map(|i| unit(*i)).collect();
    let times: Vec<f64> = (0..REPEAT)
        .map(|_| {
            let mut rt = MissionRuntime::from_source("missions/cost.lua", MISSION, 7);
            assert!(rt.errors().is_empty(), "{:?}", rt.errors());
            let mut out = Vec::new();
            let start = Instant::now();
            for tick in 0..1000u64 {
                let view = WorldView { tick, units: units.clone(), ..WorldView::default() };
                rt.think(&view, &[], &mut out);
            }
            start.elapsed().as_secs_f64() * 1e6 / 1000.0
        })
        .collect();
    println!("{:>9.1} us  per mission tick (authored script, 8 units)", median(times));
}
