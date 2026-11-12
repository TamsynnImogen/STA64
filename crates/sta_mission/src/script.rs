//! Mission scripts under STA64's Lua 5.3 (the version MMM embeds).
//!
//! [`dry_run`] loads a mission script and everything it `include()`s through the
//! VFS and runs the top level with every unknown global replaced by a recording
//! stub. It proves the script parses and runs its top level under our Lua, and
//! lists the MMM API names touched while doing so. It is the first slice of the
//! integrated runtime: the real API replaces the stubs name by name
//! (see `docs/MMM_API.md`).

use mlua::{Lua, MultiValue, Table, Value};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use sta_vfs::Vfs;

#[derive(Debug, Default)]
pub struct DryRun {
    /// Syntax and runtime errors, with the file they came from.
    pub errors: Vec<String>,
    /// API names touched at load time (`Game.hook`, `Camera()`, ...).
    pub touched: BTreeSet<String>,
    /// Files pulled in with `include()`.
    pub includes: Vec<String>,
}

type Log = Rc<RefCell<DryRun>>;

/// luabind's `class` as MMM scripts use it: `class 'Name'` makes a global class
/// whose call constructs an instance and runs `__init`; `class 'Name'(Base)`
/// inherits from `Base`. Methods are looked up through the class chain.
pub const LUABIND_CLASS: &str = r#"
function class(name)
  local c = {}
  c.__index = c
  c.__name = name
  local meta = { __call = function(cls, ...)
      local o = setmetatable({}, cls)
      if cls.__init then cls.__init(o, ...) end
      return o
    end }
  setmetatable(c, meta)
  rawset(_G, name, c)
  return function(base)
    meta.__index = base
    for k, v in pairs(base) do
      if type(k) == "string" and k:sub(1, 2) == "__" and k ~= "__index" and k ~= "__name" and rawget(c, k) == nil then
        rawset(c, k, v)
      end
    end
    return c
  end
end
"#;

/// A table that records every use: indexing, calling, arithmetic.
fn stub(lua: &Lua, name: String, log: &Log) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let mt = lua.create_table()?;
    {
        let (name, log) = (name.clone(), log.clone());
        mt.set(
            "__index",
            lua.create_function(move |lua, (_t, k): (Table, Value)| {
                let key = match &k {
                    Value::String(s) => s.to_string_lossy(),
                    other => format!("[{}]", other.type_name()),
                };
                let full = format!("{name}.{key}");
                log.borrow_mut().touched.insert(full.clone());
                stub(lua, full, &log)
            })?,
        )?;
    }
    {
        let (name, log) = (name.clone(), log.clone());
        mt.set(
            "__call",
            lua.create_function(move |lua, _args: MultiValue| {
                let full = format!("{name}()");
                log.borrow_mut().touched.insert(full.clone());
                stub(lua, full, &log)
            })?,
        )?;
    }
    for op in ["__add", "__sub", "__mul", "__div", "__mod", "__unm", "__idiv", "__pow"] {
        mt.set(op, lua.create_function(|_, _: MultiValue| Ok(0))?)?;
    }
    mt.set("__concat", lua.create_function(|_, _: MultiValue| Ok(""))?)?;
    mt.set("__lt", lua.create_function(|_, _: MultiValue| Ok(false))?)?;
    mt.set("__le", lua.create_function(|_, _: MultiValue| Ok(false))?)?;
    mt.set("__len", lua.create_function(|_, _: MultiValue| Ok(0))?)?;
    mt.set("__tostring", lua.create_function(move |_, _: MultiValue| Ok(format!("<stub {name}>")))?)?;
    t.set_metatable(Some(mt))?;
    Ok(t)
}

/// Read `missions/<name>` (or `<name>` as given) through the VFS.
fn read_script(vfs: &Vfs, name: &str) -> Option<(String, Vec<u8>)> {
    let name = name.replace('\\', "/");
    for path in [format!("missions/{name}"), name.clone()] {
        if let Some(Ok(b)) = vfs.read(&path) {
            return Some((path, b));
        }
    }
    None
}

/// Load and run `path` (a VFS path such as `missions/Eviction.lua`) with stubs.
pub fn dry_run(vfs: &Vfs, path: &str) -> DryRun {
    let log: Log = Rc::new(RefCell::new(DryRun::default()));
    let result = (|| -> mlua::Result<()> {
        let lua = crate::sandbox::new_lua();
        let globals = lua.globals();
        // Unknown globals are API stubs.
        let gmt = lua.create_table()?;
        {
            let log = log.clone();
            gmt.set(
                "__index",
                lua.create_function(move |lua, (_t, k): (Table, String)| {
                    log.borrow_mut().touched.insert(k.clone());
                    stub(lua, k, &log)
                })?,
            )?;
        }
        globals.set_metatable(Some(gmt))?;
        lua.load(LUABIND_CLASS).set_name("=luabind class").exec()?;

        // include(name): run another script from missions/ in the same state.
        let vfs_ptr = vfs as *const Vfs;
        {
            let log = log.clone();
            let include = lua.create_function(move |lua, name: String| {
                // SAFETY: the Lua state does not outlive this call to `dry_run`,
                // which holds the `&Vfs` for its whole duration.
                let vfs = unsafe { &*vfs_ptr };
                log.borrow_mut().includes.push(name.clone());
                match read_script(vfs, &name) {
                    Some((p, src)) => {
                        if let Err(e) = lua.load(&src[..]).set_mode(mlua::chunk::ChunkMode::Text).set_name(format!("@{p}")).exec() {
                            log.borrow_mut().errors.push(format!("{p}: {e}"));
                        }
                    }
                    None => log.borrow_mut().errors.push(format!("include {name}: not found")),
                }
                Ok(())
            })?;
            globals.raw_set("include", include)?;
        }

        let src = vfs.read(path).ok_or_else(|| mlua::Error::runtime(format!("{path}: not found")))?.map_err(mlua::Error::external)?;
        lua.load(&src[..]).set_mode(mlua::chunk::ChunkMode::Text).set_name(format!("@{path}")).exec()
    })();
    if let Err(e) = result {
        log.borrow_mut().errors.push(format!("{path}: {e}"));
    }
    Rc::try_unwrap(log).map(RefCell::into_inner).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn stubs_record_and_include_runs() {
        let root = std::env::temp_dir().join(format!("sta_mission_lua_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = |p: &str, s: &str| {
            let p = root.join(p);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, s).unwrap();
        };
        w("Data/missions/Main.lua", "include(\"Helper.lua\")\nlocal x = Game.time + 1\nGame.hook(\"tick\", function() end)\nHELPER_LOADED = helper()\n");
        w("Data/missions/Helper.lua", "function helper() return 3 end\n");
        w("Data/missions/Bad.lua", "if then end\n");
        w(
            "Data/missions/Classes.lua",
            "class 'Base'\nfunction Base:__init(n) self.n = n end\nfunction Base:get() return self.n end\n\
             class 'Derived'(Base)\nfunction Derived:__init(n) Base.__init(self, n * 2) end\n\
             RESULT = Derived(21):get()\nassert(RESULT == 42)\n",
        );
        let vfs = Vfs::mount_install(&root, None).unwrap();
        let r = dry_run(&vfs, "missions/Main.lua");
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.includes, ["Helper.lua"]);
        assert!(r.touched.contains("Game.hook") && r.touched.contains("Game.hook()") && r.touched.contains("Game.time"));
        assert!(!r.touched.contains("helper"), "script-defined functions are not API");
        let classes = dry_run(&vfs, "missions/Classes.lua");
        assert!(classes.errors.is_empty(), "{:?}", classes.errors);
        assert!(classes.touched.is_empty(), "classes are not API: {:?}", classes.touched);
        let bad = dry_run(&vfs, "missions/Bad.lua");
        assert_eq!(bad.errors.len(), 1, "{:?}", bad.errors);
    }

    /// With STA64_TEST_INSTALL / STA64_TEST_MOD: dry-run every mission script.
    #[test]
    fn real_missions_if_available() {
        let (Ok(root), Ok(m)) = (std::env::var("STA64_TEST_INSTALL"), std::env::var("STA64_TEST_MOD")) else { return };
        let vfs = Vfs::mount_install(Path::new(&root), Some(&m)).unwrap();
        let mut errors = Vec::new();
        let mut touched = std::collections::BTreeMap::<String, usize>::new();
        let scripts: Vec<String> = vfs.list("missions").into_iter().filter(|p| p.ends_with(".lua")).collect();
        for p in &scripts {
            let r = dry_run(&vfs, p);
            errors.extend(r.errors);
            for t in r.touched {
                *touched.entry(t).or_default() += 1;
            }
        }
        let mut top: Vec<_> = touched.into_iter().collect();
        top.sort_by_key(|e| std::cmp::Reverse(e.1));
        eprintln!("{} scripts, {} errors; top-level API: {:?}", scripts.len(), errors.len(), &top[..top.len().min(20)]);
        for e in errors.iter().take(15) {
            eprintln!("  {e}");
        }
    }
}
