//! The reproducible Lua semantics of a mission state (#1300, `docs/MISSIONS.md` "Determinism"):
//! `pairs`/`next` in insertion order, a stable `table.sort`, no weak tables.

use mlua::{Lua, Table, Value};
use sta_mission::sandbox::new_lua;

fn eval(lua: &Lua, chunk: &str) -> String {
    sta_mission::sandbox::refill(lua);
    lua.load(chunk).eval::<String>().unwrap_or_else(|e| panic!("{chunk}: {e}"))
}

/// Keys of every type a script can use come out in the order they were added.
#[test]
fn pairs_and_next_follow_insertion_order_for_every_key_type() {
    let lua = new_lua();
    let ud: Vec<_> = (0..4u32).map(|i| lua.create_any_userdata(i).unwrap()).collect();
    lua.globals().set("UD", ud).unwrap();
    let chunk = r#"
        local K = { "s1", {}, function() end, coroutine.create(function() end), true, 2.5, -7,
                    2^53, "s2", UD[3], false, 0.1, UD[1], {}, 1e300, "", -1.5, math.mininteger, UD[2] }
        local T = {}
        for i, k in ipairs(K) do T[k] = i end
        local a, b = {}, {}
        for _, i in pairs(T) do a[#a + 1] = i end
        local k, v = next(T)
        while k ~= nil do b[#b + 1] = v; k, v = next(T, k) end
        return table.concat(a, ",") .. "|" .. table.concat(b, ",")
    "#;
    let order: Vec<String> = (1..=19).map(|i| i.to_string()).collect();
    let order = order.join(",");
    assert_eq!(eval(&lua, chunk), format!("{order}|{order}"));
}

/// The table constructor adds its named fields in source order; tables grown past many
/// rehashes keep the order.
#[test]
fn constructors_and_growth_keep_the_order() {
    let lua = new_lua();
    assert_eq!(eval(&lua, "local t = { z = 1, y = 2, x = 3, w = 4 } local o = {} for k in pairs(t) do o[#o+1] = k end return table.concat(o)"), "zyxw");
    let chunk = "local t = {} for i = 1, 5000 do t['k' .. (i * 7919 % 5003)] = i end \
                 local prev, ok = 0, true for _, v in pairs(t) do ok = ok and v == prev + 1; prev = v end return tostring(ok and prev == 5000)";
    assert_eq!(eval(&lua, chunk), "true");
}

/// Clearing fields during a traversal stays legal; a key cleared and added again moves to the end,
/// also when the collector ran in between and after the table grew.
#[test]
fn clearing_during_traversal_and_re_adding() {
    let lua = new_lua();
    let chunk = "local t = { a = 1, b = 2, c = 3, d = 4 } local seen = {} \
                 for k, v in pairs(t) do seen[#seen+1] = k; if v % 2 == 0 then t[k] = nil else t[k] = v * 10 end end \
                 local rest = {} for k, v in pairs(t) do rest[#rest+1] = k .. v end \
                 return table.concat(seen) .. '|' .. table.concat(rest, ',')";
    assert_eq!(eval(&lua, chunk), "abcd|a10,c30");
    eval(&lua, "T = { a = 1, b = 2, c = 3 } T.a = nil return ''");
    lua.gc_collect().unwrap();
    assert_eq!(eval(&lua, "T.a = 9 local o = {} for k in pairs(T) do o[#o+1] = k end return table.concat(o)"), "bca");
    let chunk = "local t = { a = 1, b = 2, c = 3 } t.b = nil for i = 1, 100 do t['g' .. i] = i end t.b = 2 \
                 local o = {} for k in pairs(t) do if #k == 1 then o[#o+1] = k end end return table.concat(o)";
    assert_eq!(eval(&lua, chunk), "acb");
    // rawset obeys the same rule; overwriting a present key keeps its place.
    assert_eq!(eval(&lua, "local t = { a = 1, b = 2 } rawset(t, 'a', nil) rawset(t, 'a', 1) t.b = 5 local o = {} for k in pairs(t) do o[#o+1] = k end return table.concat(o)"), "ba");
}

/// Undefined in Lua (assigning a key that is absent during a traversal), but the same in every
/// state: whether the collector ran in between does not matter.
#[test]
fn re_adding_the_current_key_during_traversal_is_the_same_with_and_without_a_collection() {
    let chunk = "local o = {} for k, v in pairs(T) do o[#o+1] = k if k == 'b' then T.b = nil; collect(); T.b = v end end return table.concat(o)";
    let mut results = Vec::new();
    for gc in [false, true] {
        let lua = new_lua();
        let collect = lua
            .create_function(move |lua, ()| {
                if gc {
                    lua.gc_collect()?;
                }
                Ok(())
            })
            .unwrap();
        lua.globals().set("collect", collect).unwrap();
        eval(&lua, "T = { a = 1, b = 2, c = 3 } return ''");
        results.push(eval(&lua, chunk));
    }
    assert_eq!(results[0], results[1]);
}

/// The list part comes first, in ascending order; other keys follow in insertion order.
#[test]
fn array_part_first_then_insertion_order() {
    let lua = new_lua();
    assert_eq!(eval(&lua, "local t = {} for i = 1, 8 do t[i] = i end local o = {} for k in pairs(t) do o[#o+1] = k end return table.concat(o, ',')"), "1,2,3,4,5,6,7,8");
    assert_eq!(eval(&lua, "local t = { 'a', 'b', name = 'n', 'c', id = 1 } local o = {} for k in pairs(t) do o[#o+1] = tostring(k) end return table.concat(o, ',')"), "1,2,3,name,id");
    // Sparse integers live in the hash part: insertion order.
    assert_eq!(eval(&lua, "local t = {} for _, k in ipairs({ 900, 30, 7000, -2 }) do t[k] = true end local o = {} for k in pairs(t) do o[#o+1] = k end return table.concat(o, ',')"), "900,30,7000,-2");
}

/// The guarantee for integer keys, as documented: a list built by appending or by a constructor
/// comes first in ascending order, and every key that is not a positive integer keeps insertion
/// order relative to the others. Where any other positive integer key appears is not specified,
/// but it is the same for the same operations, in every state (checked with the heap perturbed).
#[test]
fn integer_keys_are_deterministic_and_lists_come_first() {
    let chunk = r#"
        local out = {}
        local function walk(t) local o = {} for k in pairs(t) do o[#o+1] = tostring(k) end return table.concat(o, ',') end
        -- lists built by appending, interleaved with other keys
        local a = { x = 1 } for i = 1, 20 do a[#a + 1] = i; a['s' .. i] = i end table.insert(a, 21)
        out[#out+1] = walk(a)
        -- sparse, out of order, zero, negative, float keys with integer values, huge integers
        local b = {} for _, k in ipairs({ 17, 3, 0, -4, 1, 2.0, 9, 2^31 + 5, 5.0, -0.0, 4, 100, 6 }) do b[k] = k end
        out[#out+1] = walk(b)
        -- keys moving between the parts at rebuilds: fill, clear most, grow again
        local c = {} for i = 1, 64 do c[i] = i end for i = 1, 64 do if i % 5 ~= 0 then c[i] = nil end end
        for i = 1, 40 do c['g' .. i] = i end for i = 65, 80 do c[i] = i end
        out[#out+1] = walk(c)
        -- insert and remove in the middle of a list with other keys present
        local d = { 'a', 'b', 'c', 'd', name = 'n' } table.insert(d, 2, 'x') table.remove(d, 4) d.tag = 1 table.insert(d, 1, 'y')
        out[#out+1] = walk(d)
        -- a list filled in order around keys that were there first
        local e = { k = 1 } e[12] = 12 e.m = 2 for i = 1, 11 do e[i] = i end
        out[#out+1] = walk(e)
        return table.concat(out, '|')
    "#;
    let mut runs = Vec::new();
    for variant in 0..4usize {
        let mut held: Vec<Box<[u8]>> = Vec::new();
        for i in 0..variant * 2000 {
            held.push(vec![0u8; 8 + (i * 53) % 900].into_boxed_slice());
        }
        let lua = new_lua();
        runs.push(eval(&lua, chunk));
        drop(held);
    }
    assert!(runs.iter().all(|r| *r == runs[0]), "{runs:#?}");
    let parts: Vec<&str> = runs[0].split('|').collect();
    // the appended list first, ascending; then x, s1..s20 in insertion order
    let list: Vec<String> = (1..=21).map(|i| i.to_string()).collect();
    let rest: Vec<String> = std::iter::once("x".to_string()).chain((1..=20).map(|i| format!("s{i}"))).collect();
    assert_eq!(parts[0], format!("{},{}", list.join(","), rest.join(",")));
    // non-positive and huge keys keep insertion order among themselves
    let b: Vec<&str> = parts[1].split(',').filter(|k| ["0", "-4", "2147483653"].contains(k)).collect();
    assert_eq!(b, ["0", "-4", "2147483653"], "{}", parts[1]);
    // the g keys keep insertion order among themselves
    let g: Vec<&str> = parts[2].split(',').filter(|k| k.starts_with('g')).collect();
    let want: Vec<String> = (1..=40).map(|i| format!("g{i}")).collect();
    assert_eq!(g, want);
    assert_eq!(parts[3], "1,2,3,4,5,name,tag");
    assert_eq!(parts[4], "1,2,3,4,5,6,7,8,9,10,11,12,k,m");
}

/// The host cannot smuggle a weak table in either: the bundled Lua ignores `__mode`.
#[test]
fn a_host_made_weak_table_stays_strong() {
    let lua = new_lua();
    let weak = lua.create_table().unwrap();
    let meta = lua.create_table().unwrap();
    meta.set("__mode", "kv").unwrap();
    weak.set_metatable(Some(meta)).unwrap();
    for _ in 0..50 {
        weak.set(lua.create_table().unwrap(), lua.create_table().unwrap()).unwrap();
    }
    lua.gc_collect().unwrap();
    lua.gc_collect().unwrap();
    assert_eq!(weak.pairs::<Value, Value>().count(), 50);
}

#[test]
fn ipairs_len_table_functions_and_pairs_metamethod_are_unchanged() {
    let lua = new_lua();
    assert_eq!(eval(&lua, "local t = { 1, 2, 3 } table.insert(t, 4) table.insert(t, 1, 0) table.remove(t, 2) local o = {} for _, v in ipairs(t) do o[#o+1] = v end return #t .. ':' .. table.concat(o, ',') .. ':' .. table.concat(table.move(t, 1, 2, 1, {}), ',') .. ':' .. select('#', table.unpack(t))"), "4:0,2,3,4:0,2:4");
    let chunk = "local t = setmetatable({}, { __pairs = function(t) return function(_, k) if not k then return 1, 'x' end end, t, nil end }) \
                 local o = {} for k, v in pairs(t) do o[#o+1] = k .. v end return table.concat(o)";
    assert_eq!(eval(&lua, chunk), "1x");
}

/// Tables the host builds and walks follow the same order.
#[test]
fn host_built_tables_and_host_iteration_use_the_same_order() {
    let lua = new_lua();
    let t: Table = lua.create_table().unwrap();
    for k in ["zeta", "alpha", "mid", "beta"] {
        t.set(k, k.len()).unwrap();
    }
    let host: Vec<String> = t.pairs::<String, i64>().map(|p| p.unwrap().0).collect();
    assert_eq!(host, ["zeta", "alpha", "mid", "beta"]);
    lua.globals().set("H", t).unwrap();
    assert_eq!(eval(&lua, "local o = {} for k in pairs(H) do o[#o+1] = k end return table.concat(o, ',')"), "zeta,alpha,mid,beta");
}

#[test]
fn sort_is_stable_and_never_reads_the_clock() {
    let lua = new_lua();
    // Ties keep their original order, also on the inputs that made the old sort pick a clock pivot.
    let chunk = "local a = {} for i = 1, 1000 do a[i] = { id = i, key = i % 3 + 1 } end a[1].key = 0 a[500].key = 0 a[1000].key = 9 \
                 table.sort(a, function(x, y) return x.key < y.key end) \
                 for i = 2, #a do local p, q = a[i-1], a[i] if p.key > q.key or (p.key == q.key and p.id > q.id) then return 'unstable at ' .. i end end return 'ok'";
    assert_eq!(eval(&lua, chunk), "ok");
    // Without ties the result is the sorted order, as before; strings by byte order.
    assert_eq!(eval(&lua, "local a = { 5, 3.5, -1, 10, 2^60, 0 } table.sort(a) return table.concat(a, ',')"), "-1,0,3.5,5,10,1.1529215046068e+18");
    assert_eq!(eval(&lua, "local a = { 'b', 'B', 'a', 'ab', '' } table.sort(a) return table.concat(a, ',')"), ",B,a,ab,b");
    assert_eq!(eval(&lua, "local a = { 3, 1, 2 } table.sort(a, function(x, y) return x > y end) return table.concat(a, ',')"), "3,2,1");
}

#[test]
fn sort_errors_are_defined() {
    let lua = new_lua();
    // A comparator that is not a strict weak order: no error, the same elements.
    let chunk = "local a = {} for i = 1, 300 do a[i] = i % 7 end \
                 local ok1 = pcall(table.sort, a, function() return true end) \
                 local ok2 = pcall(table.sort, a, function(x, y) return x <= y end) \
                 local n = {} for _, v in ipairs(a) do n[v] = (n[v] or 0) + 1 end \
                 return tostring(ok1) .. tostring(ok2) .. #a .. ':' .. n[0] .. ',' .. n[6]";
    assert_eq!(eval(&lua, chunk), "truetrue300:42,43");
    // A raising comparator: its error value comes through unchanged and the table is untouched.
    let chunk = "local a = { 3, 1, 2 } local e = {} local ok, err = pcall(table.sort, a, function() error(e) end) \
                 return tostring(ok) .. tostring(err == e) .. table.concat(a, ',')";
    assert_eq!(eval(&lua, chunk), "falsetrue3,1,2");
    let err = lua.load("table.sort({ 1, 'x' })").exec().unwrap_err().to_string();
    assert!(err.contains("attempt to compare"), "{err}");
    let err = lua.load("table.sort({ 2, 1 }, 5)").exec().unwrap_err().to_string();
    assert!(err.contains("bad argument #2 to 'sort'"), "{err}");
}

/// The workspace builds the patched Lua (vendor/lua-src). Fails if `[patch.crates-io]` ever
/// falls back to the registry crate.
#[test]
fn the_bundled_lua_is_the_patched_copy() {
    let lua = Lua::new();
    let order = lua
        .load("local t = {} for i = 1, 200 do t['k' .. (i * 37 % 211)] = i end local p = 0 for _, v in pairs(t) do if v ~= p + 1 then return false end p = v end return true")
        .eval::<bool>()
        .unwrap();
    assert!(order, "pairs is not in insertion order: the unpatched Lua was built");
}

#[test]
fn weak_tables_are_refused_and_a_late_mode_is_ignored() {
    let lua = new_lua();
    let err = lua.load("setmetatable({}, { __mode = 'k' })").exec().unwrap_err().to_string();
    assert!(err.contains("__mode") && err.contains("reproducible"), "{err}");
    eval(&lua, "local mt = {} W = setmetatable({}, mt) mt.__mode = 'kv' for i = 1, 100 do W[{}] = {} end return ''");
    lua.gc_collect().unwrap();
    lua.gc_collect().unwrap();
    assert_eq!(eval(&lua, "local n = 0 for _ in pairs(W) do n = n + 1 end return tostring(n)"), "100");
    assert!(matches!(lua.globals().get::<Value>("W").unwrap(), Value::Table(_)));
}
