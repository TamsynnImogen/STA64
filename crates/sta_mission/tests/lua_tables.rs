//! Table internals of the patched Lua (vendor/lua-src, #1300): resizes that never nest (memory
//! safety), a differential test of the documented semantics against a model, bounded memory for
//! queue and registry idioms, amortised rebuild cost, the memory limit, and the same traversal in
//! every state when keys are added during it.

use mlua::{Function, Lua, Table, Value};
use sta_mission::sandbox::{new_lua, refill};

fn eval<T: mlua::FromLuaMulti>(lua: &Lua, chunk: &str) -> T {
    refill(lua);
    lua.load(chunk).eval::<T>().unwrap_or_else(|e| panic!("{chunk}: {e}"))
}

/// Slots the patched Lua has visited in resizes, rebuilds and density checks so far.
fn table_work(lua: &Lua) -> i64 {
    unsafe extern "C" {
        fn lua_sta64_tablework(state: *mut std::ffi::c_void) -> i64;
    }
    let mut work = 0;
    // SAFETY: the callback only reads a counter of the state it is given.
    unsafe { lua.exec_raw::<()>((), |state| work = lua_sta64_tablework(state.cast())) }.unwrap();
    work
}

/// A shrinking rebuild re-inserts the slice of the array part that goes away; that must never
/// grow or rebuild the table from inside (it lost values and wrote past the array).
#[test]
fn a_shrinking_rebuild_keeps_every_value() {
    let lua = new_lua();
    assert_eq!(eval::<i64>(&lua, "local t = {1, 2, 3, nil, 5} t.a = true return t[5]"), 5);
    let chunk = "local t = {1, 2, 3, nil, 5} t.a = true for i = 5, 8 do t[i] = i end \
                 local s = 0 for i = 1, 8 do s = s + (t[i] or 0) end return s";
    assert_eq!(eval::<i64>(&lua, chunk), 1 + 2 + 3 + 5 + 6 + 7 + 8);
    let chunk = "local t = {} for i = 1, 8 do t[i] = i end t[4], t[6], t[7], t[8] = nil, nil, nil, nil t.a = 1 \
                 return (t[1] or 0) + (t[2] or 0) + (t[3] or 0) + (t[5] or 0) + (t[4] or 0) * 100";
    assert_eq!(eval::<i64>(&lua, chunk), 11);
}

/// The family around that case: constructors with holes, then shrinking or growing rebuilds and
/// mixed parts, at sizes around every power of two up to 4096. Every value must survive.
#[test]
fn rebuilds_around_every_power_of_two_keep_every_value() {
    let lua = new_lua();
    let check: Function = lua
        .load(
            r#"
        return function(n, hole, extra)
          -- a constructor of n items with every 'hole'-th one missing
          local t = load_list(n, hole)
          local want = {}
          for i = 1, n do if i % hole ~= 0 then want[i] = i end end
          -- string keys force rebuilds; integer keys around the array end grow and shrink it
          for j = 1, extra do t['s' .. j] = j; want['s' .. j] = j end
          for i = n + 1, n + extra do t[i] = i; want[i] = i end
          for i = 1, n, 3 do t[i] = nil; want[i] = nil end
          for j = 1, extra, 2 do t['s' .. j] = nil; want['s' .. j] = nil end
          for j = 1, extra do t['r' .. j] = -j; want['r' .. j] = -j end
          for k, v in pairs(want) do if t[k] ~= v then return 'lost ' .. tostring(k) end end
          local n2 = 0
          for k, v in pairs(t) do n2 = n2 + 1; if want[k] ~= v then return 'extra ' .. tostring(k) end end
          local n1 = 0 for _ in pairs(want) do n1 = n1 + 1 end
          if n1 ~= n2 then return 'count ' .. n1 .. ' ' .. n2 end
          return 'ok'
        end
    "#,
        )
        .eval()
        .unwrap();
    // A real constructor (OP_NEWTABLE + SETLIST with nils) of n items, compiled by the host.
    let load_list = lua
        .create_function(move |lua, (n, hole): (i64, i64)| {
            let items: Vec<String> = (1..=n).map(|i| if i % hole == 0 { "nil".into() } else { i.to_string() }).collect();
            lua.load(format!("return {{{}}}", items.join(","))).eval::<Table>()
        })
        .unwrap();
    lua.globals().set("load_list", load_list).unwrap();
    let mut sizes: Vec<i64> = Vec::new();
    for p in 0..=12 {
        let b = 1i64 << p;
        sizes.extend([b - 1, b, b + 1]);
    }
    sizes.retain(|n| *n >= 1);
    for n in sizes {
        for hole in [2, 3, 5, 1000] {
            for extra in [1, 3, 17] {
                refill(&lua);
                let got: String = check.call((n, hole, extra)).unwrap();
                assert_eq!(got, "ok", "n {n} hole {hole} extra {extra}");
            }
        }
    }
}

/// Tiny deterministic generator for the differential test.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The model of the documented semantics: the list 1..n (kept by list operations only), then
/// every other key in the order it was added, a key re-added after nil going to the end.
#[derive(Default)]
struct Model {
    list: Vec<i64>,
    other: Vec<(usize, i64)>,
}

impl Model {
    fn set_other(&mut self, key: usize, value: Option<i64>) {
        let at = self.other.iter().position(|(k, _)| *k == key);
        match (at, value) {
            (Some(i), Some(v)) => self.other[i].1 = v,
            (None, Some(v)) => self.other.push((key, v)),
            (Some(i), None) => {
                self.other.remove(i);
            }
            (None, None) => {}
        }
    }
}

/// Random sequences of table operations against the model: every lookup, the length, and the
/// traversal order (with clears and additions during the traversal) must agree. Fixed seeds.
#[test]
fn random_operation_sequences_match_the_documented_semantics() {
    let lua = new_lua();
    // Keys besides the list: short and long strings, zero, negatives, non-integral floats,
    // booleans, tables, functions, and sparse positive integers far from the list (the docs
    // leave the place of sparse positive integers unspecified; far from a list the patched
    // Lua keeps them in insertion order, which this test pins).
    let pool: Table = lua
        .load(
            "local p = {} for i = 1, 24 do p[#p+1] = 'k' .. i end \
             for i = 1, 4 do p[#p+1] = string.rep('long key ', 8) .. i end \
             for _, v in ipairs({ 0, -1, -7, -1000000, 2.5, -0.5, 1e300 }) do p[#p+1] = v end \
             for i = 1, 5 do p[#p+1] = 1000000 + 7919 * i end \
             p[#p+1] = true p[#p+1] = false \
             for i = 1, 6 do p[#p+1] = {} end for i = 1, 4 do p[#p+1] = function() return i end end return p",
        )
        .eval()
        .unwrap();
    let keys: Vec<Value> = pool.sequence_values::<Value>().map(Result::unwrap).collect();
    let insert: Function = lua.load("return table.insert").eval().unwrap();
    let remove: Function = lua.load("return table.remove").eval().unwrap();
    let mut checked = 0u64;
    for seed in 1..=3000u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let t = lua.create_table().unwrap();
        let mut m = Model::default();
        let steps = 40 + rng.below(160) as i64;
        for v in 2..steps + 2 {
            refill(&lua);
            match rng.below(12) {
                0 | 1 => {
                    let k = rng.below(keys.len() as u64) as usize;
                    t.set(keys[k].clone(), v).unwrap();
                    m.set_other(k, Some(v));
                }
                2 => {
                    let k = rng.below(keys.len() as u64) as usize;
                    t.raw_set(keys[k].clone(), v).unwrap();
                    m.set_other(k, Some(v));
                }
                3 | 4 => {
                    let k = rng.below(keys.len() as u64) as usize;
                    t.set(keys[k].clone(), Value::Nil).unwrap();
                    m.set_other(k, None);
                }
                5 | 6 => {
                    t.set(m.list.len() as i64 + 1, v).unwrap();
                    m.list.push(v);
                }
                7 => {
                    let pos = rng.below(m.list.len() as u64 + 1) as i64 + 1;
                    insert.call::<()>((&t, pos, v)).unwrap();
                    m.list.insert(pos as usize - 1, v);
                }
                8 if !m.list.is_empty() => {
                    let pos = rng.below(m.list.len() as u64) as i64 + 1;
                    let got: i64 = remove.call((&t, pos)).unwrap();
                    assert_eq!(got, m.list.remove(pos as usize - 1), "seed {seed}");
                }
                9 if !m.list.is_empty() => {
                    let i = rng.below(m.list.len() as u64) as usize;
                    t.set(i as i64 + 1, v).unwrap();
                    m.list[i] = v;
                }
                10 if !m.list.is_empty() => {
                    t.set(m.list.len() as i64, Value::Nil).unwrap();
                    m.list.pop();
                }
                _ => {
                    // Traverse; at a visited key not in the list, clear it (a third of the
                    // time) or add a key that is absent (a sixth): documented, an added key
                    // goes to the end and is visited (never both at one key, so the current
                    // key is never one a rebuild drops).
                    let mut want: Vec<String> = (1..=m.list.len() as i64).map(|i| format!("{:?}", Value::Integer(i))).collect();
                    want.extend(m.other.iter().map(|(k, _)| format!("{:?}", keys[*k])));
                    let mut seen: Vec<String> = Vec::new();
                    for pair in t.pairs::<Value, Value>() {
                        let (k, _) = pair.unwrap();
                        seen.push(format!("{k:?}"));
                        let Some(at) = keys.iter().position(|p| p == &k) else { continue };
                        match rng.below(6) {
                            0 | 1 => {
                                t.set(k, Value::Nil).unwrap();
                                m.set_other(at, None);
                            }
                            2 => {
                                let add = rng.below(keys.len() as u64) as usize;
                                if !m.other.iter().any(|(o, _)| *o == add) {
                                    t.set(keys[add].clone(), -v).unwrap();
                                    m.set_other(add, Some(-v));
                                    want.push(format!("{:?}", keys[add]));
                                }
                            }
                            _ => {}
                        }
                    }
                    assert_eq!(seen, want, "seed {seed}: traversal order");
                }
            }
            assert_eq!(t.raw_len(), m.list.len(), "seed {seed}: length");
            for (i, want) in m.list.iter().enumerate() {
                assert_eq!(t.raw_get::<i64>(i as i64 + 1).unwrap(), *want, "seed {seed}: list item {}", i + 1);
            }
            for (k, key) in keys.iter().enumerate() {
                let want = m.other.iter().find(|(o, _)| *o == k).map(|(_, v)| *v);
                assert_eq!(t.raw_get::<Option<i64>>(key.clone()).unwrap(), want, "seed {seed}: key {key:?}");
            }
            checked += 1;
        }
    }
    assert!(checked > 300_000, "{checked} operations checked");
}

/// Sparse and out-of-order positive integer keys: the place is unspecified, but every value is
/// found, nothing is visited twice, and the order is the same in states with other heaps.
#[test]
fn random_integer_keys_are_found_and_ordered_the_same_way_in_every_state() {
    let chunk = r#"
        local out, rng = {}, 12345
        local function r(n) rng = (rng * 1103515245 + 12345) % 2147483648 return rng % n end
        for s = 1, 300 do
          local t, want = {}, {}
          for _ = 1, 60 + r(100) do
            local k = (r(4) == 0) and ('s' .. r(30)) or (r(3) == 0 and r(2000) - 10 or r(70) + 1)
            if r(4) == 0 then t[k] = nil want[k] = nil else t[k] = s want[k] = s end
          end
          local o, n = {}, 0
          for k, v in pairs(t) do
            if want[k] ~= v or o[k] then return 'bad ' .. s .. ' ' .. tostring(k) end
            o[k] = true n = n + 1 out[#out+1] = tostring(k)
          end
          for k in pairs(want) do if not o[k] then return 'missing ' .. tostring(k) end end
        end
        return table.concat(out, ',')
    "#;
    let mut runs = Vec::new();
    for variant in 0..3usize {
        let held: Vec<Box<[u8]>> = (0..variant * 3000).map(|i| vec![0u8; 8 + (i * 61) % 800].into_boxed_slice()).collect();
        let lua = new_lua();
        runs.push(eval::<String>(&lua, chunk));
        drop(held);
    }
    assert!(!runs[0].starts_with("bad") && !runs[0].starts_with("missing"), "{}", &runs[0][..80.min(runs[0].len())]);
    assert!(runs.iter().all(|r| *r == runs[0]));
}

/// A queue and a registry with ever-increasing ids keep memory proportional to the live entries.
#[test]
fn queues_and_registries_keep_memory_flat() {
    let lua = new_lua();
    eval::<()>(
        &lua,
        "Q, H, T = {}, 1, 0 function step(n) for _ = 1, n do T = T + 1 Q[T] = T if T - H > 50 then Q[H] = nil H = H + 1 end end end \
         R, ID = { [1] = 'permanent' }, 1 function reg(n) for _ = 1, n do ID = ID + 1 R[ID] = ID R[ID - 40] = nil end end",
    );
    eval::<()>(&lua, "step(5000) reg(5000)");
    lua.gc_collect().unwrap();
    let early = lua.used_memory();
    for _ in 0..20 {
        eval::<()>(&lua, "step(10000) reg(10000)");
    }
    lua.gc_collect().unwrap();
    let late = lua.used_memory();
    assert!(late < early + 64 * 1024, "memory grew from {early} to {late} bytes over 200 000 pushes");
}

/// Toggling keys and churning small keys next to a big list costs amortised constant work, not
/// a pass over the list per assignment.
#[test]
fn rebuild_cost_is_amortised_next_to_a_big_list() {
    let lua = new_lua();
    eval::<()>(&lua, "T = {} for i = 1, 200000 do T[i] = i end T.a = 1");
    let before = table_work(&lua);
    eval::<()>(&lua, "for i = 1, 20000 do if i % 2 == 0 then T.a = nil T.b = 1 else T.b = nil T.a = 1 end end");
    eval::<()>(&lua, "for i = 1, 20000 do T['c' .. i] = i T['c' .. (i - 3)] = nil end");
    let work = table_work(&lua) - before;
    assert!(work < 40 * 40000, "{work} slots visited for 40 000 assignments next to a 200 000-item list");
}

/// Growing and shrinking tables up to the memory limit ends in the memory error, every time.
#[test]
fn the_memory_limit_ends_growth_cleanly() {
    let lua = new_lua();
    lua.set_memory_limit(8 * 1024 * 1024).unwrap();
    let grow = "KEEP = {} while true do local t = {} for i = 1, 300 do t[i] = i t['k' .. i] = i end \
                for i = 1, 300, 2 do t[i] = nil t['k' .. i] = nil end t[#t + 1] = 1 KEEP[#KEEP + 1] = t end";
    for round in 0..6 {
        refill(&lua);
        let err = lua.load(grow).exec().unwrap_err().to_string();
        assert!(err.to_lowercase().contains("memory"), "round {round}: {err}");
        lua.globals().set("KEEP", Value::Nil).unwrap();
        lua.gc_collect().unwrap();
        assert_eq!(eval::<i64>(&lua, "local t = {} for i = 1, 100 do t[i] = i end return #t"), 100);
    }
}

/// Adding keys during a traversal is undefined in Lua; here it must still be the same in every
/// state, whatever the per-state hash seed.
#[test]
fn adding_keys_during_a_traversal_is_the_same_in_every_state() {
    let chunk = "local out = {} for size = 1, 12 do local t = {} for i = 1, size do t['k' .. i] = i end local n = 0 \
                 local ok = pcall(function() for k in pairs(t) do n = n + 1 if n > 100 then break end t[k] = nil t[k .. 'x'] = true end end) \
                 out[#out+1] = n .. tostring(ok) end return table.concat(out, ',')";
    let first = eval::<String>(&new_lua(), chunk);
    for _ in 0..300 {
        assert_eq!(eval::<String>(&new_lua(), chunk), first);
    }
}
