//! Deterministic `math.*` for mission scripts (see `docs/MISSIONS.md`, "Determinism").
//!
//! Lua's `+ - * /`, `math.floor`, `math.ceil`, `math.sqrt`, `math.abs`
//! are IEEE-exact and identical everywhere. The transcendental functions call the
//! platform libm, whose last bits differ, so [`install`] replaces them with the
//! pure-Rust [`libm`] crate (pinned `=0.2.16`): fixed algorithms in plain f64
//! arithmetic, no platform libm. libm may use hardware FMA only where the build
//! target enables it (aarch64), and a fused multiply-add is correctly rounded
//! exactly like libm's software `fma`, so results match on all targets.
//! The golden tests pin exact bit patterns and run on the 4-OS CI matrix.
//!
//! `math.random` is xoshiro256** seeded through splitmix64, with Lua 5.3/5.4
//! argument semantics: `()` gives `[0,1)`, `(m)` gives `[1,m]`, `(m,n)` gives
//! `[m,n]`; integers are drawn by rejection sampling (no modulo bias).
//!
//! Limit: the `^` operator is compiled into Lua's VM and calls C `pow`; it cannot
//! be replaced from the `math` table. Scripts that need cross-platform exactness
//! must use `math.pow(x, y)` (provided here although Lua 5.3 dropped it).

use mlua::{Lua, MultiValue, Result, Table, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// xoshiro256** generator state.
#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
}

fn splitmix64(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut x = seed;
        Rng { s: [splitmix64(&mut x), splitmix64(&mut x), splitmix64(&mut x), splitmix64(&mut x)] }
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let r = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        r
    }

    /// Uniform in `[0,1)` with 53 random bits.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }

    /// Uniform integer in `[lo, hi]`, `lo <= hi`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        let span = (hi as u64).wrapping_sub(lo as u64);
        if span == u64::MAX {
            return self.next_u64() as i64;
        }
        let n = span + 1;
        // Reject the biased tail of the 64-bit range.
        let zone = u64::MAX - (u64::MAX % n + 1) % n;
        loop {
            let r = self.next_u64();
            if r <= zone {
                return (lo as u64).wrapping_add(r % n) as i64;
            }
        }
    }
}

fn num(v: &Value, what: &str) -> Result<f64> {
    match v {
        Value::Number(n) => Ok(*n),
        Value::Integer(i) => Ok(*i as f64),
        Value::String(s) => s
            .to_str()?
            .trim()
            .parse::<f64>()
            .map_err(|_| mlua::Error::runtime(format!("bad argument to '{what}' (number expected)"))),
        _ => Err(mlua::Error::runtime(format!("bad argument to '{what}' (number expected)"))),
    }
}

fn int(v: &Value, what: &str) -> Result<i64> {
    match v {
        Value::Integer(i) => Ok(*i),
        _ => {
            let n = num(v, what)?;
            if n.fract() == 0.0 && (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&n) {
                Ok(n as i64)
            } else {
                Err(mlua::Error::runtime(format!(
                    "bad argument to '{what}' (number has no integer representation)"
                )))
            }
        }
    }
}

fn arg(a: &MultiValue, i: usize) -> Value {
    a.get(i).cloned().unwrap_or(Value::Nil)
}

fn set1(lua: &Lua, m: &Table, name: &'static str, f: fn(f64) -> f64) -> Result<()> {
    m.set(name, lua.create_function(move |_, a: MultiValue| Ok(f(num(&arg(&a, 0), name)?)))?)
}

fn set2(lua: &Lua, m: &Table, name: &'static str, f: fn(f64, f64) -> f64) -> Result<()> {
    m.set(
        name,
        lua.create_function(move |_, a: MultiValue| Ok(f(num(&arg(&a, 0), name)?, num(&arg(&a, 1), name)?)))?,
    )
}

/// Replace the platform-dependent functions of the state's `math` table.
pub fn install(lua: &Lua, seed: u64) -> Result<()> {
    let m: Table = lua.globals().get("math")?;
    set1(lua, &m, "sin", libm::sin)?;
    set1(lua, &m, "cos", libm::cos)?;
    set1(lua, &m, "tan", libm::tan)?;
    set1(lua, &m, "asin", libm::asin)?;
    set1(lua, &m, "acos", libm::acos)?;
    set1(lua, &m, "exp", libm::exp)?;
    set2(lua, &m, "pow", libm::pow)?;
    set2(lua, &m, "fmod", libm::fmod)?;
    // atan(y [, x]) is atan2(y, x), x defaulting to 1.
    m.set(
        "atan",
        lua.create_function(|_, a: MultiValue| {
            let y = num(&arg(&a, 0), "atan")?;
            let x = match arg(&a, 1) {
                Value::Nil => 1.0,
                v => num(&v, "atan")?,
            };
            Ok(libm::atan2(y, x))
        })?,
    )?;
    // log(x [, base]); bases 2 and 10 use the dedicated functions like Lua does.
    m.set(
        "log",
        lua.create_function(|_, a: MultiValue| {
            let x = num(&arg(&a, 0), "log")?;
            Ok(match arg(&a, 1) {
                Value::Nil => libm::log(x),
                v => {
                    let b = num(&v, "log")?;
                    if b == 2.0 {
                        libm::log2(x)
                    } else if b == 10.0 {
                        libm::log10(x)
                    } else {
                        libm::log(x) / libm::log(b)
                    }
                }
            })
        })?,
    )?;

    let rng = Rc::new(RefCell::new(Rng::new(seed)));
    let r = rng.clone();
    m.set(
        "random",
        lua.create_function(move |_, a: MultiValue| {
            let mut g = r.borrow_mut();
            let (lo, hi) = match a.len() {
                0 => return Ok(Value::Number(g.next_f64())),
                1 => (1, int(&arg(&a, 0), "random")?),
                2 => (int(&arg(&a, 0), "random")?, int(&arg(&a, 1), "random")?),
                _ => return Err(mlua::Error::runtime("wrong number of arguments")),
            };
            if lo > hi {
                return Err(mlua::Error::runtime("bad argument to 'random' (interval is empty)"));
            }
            Ok(Value::Integer(g.range(lo, hi)))
        })?,
    )?;
    m.set(
        "randomseed",
        lua.create_function(move |_, a: MultiValue| {
            let s = match arg(&a, 0) {
                Value::Integer(i) => i as u64,
                v => num(&v, "randomseed")?.to_bits(),
            };
            *rng.borrow_mut() = Rng::new(s);
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const XS: [f64; 12] = [0.0, 0.1, 0.5, 1.0, 2.0, std::f64::consts::PI, 10.0, 100.0, 1e-5, 0.9, -0.7, 123.456];

    const CASES: [(&str, &str); 11] = [
        ("sin", "math.sin(x)"),
        ("cos", "math.cos(x)"),
        ("tan", "math.tan(x)"),
        ("asin", "math.asin(x - math.floor(x))"),
        ("acos", "math.acos(x - math.floor(x))"),
        ("atan", "math.atan(x, 0.3)"),
        ("exp", "math.exp(x / 7)"),
        ("log", "math.log(math.abs(x) + 1)"),
        ("log2", "math.log(math.abs(x) + 1, 2)"),
        ("pow", "math.pow(math.abs(x) + 1, 0.37)"),
        ("fmod", "math.fmod(x, 0.7)"),
    ];

    fn state() -> Lua {
        let lua = crate::sandbox::new_lua();
        install(&lua, 12345).unwrap();
        lua.globals().set("xs", XS.to_vec()).unwrap();
        lua
    }

    fn run(lua: &Lua, f: &str) -> Vec<u64> {
        let src = format!("local r = {{}} for _, x in ipairs(xs) do r[#r+1] = {f} end return r");
        lua.load(&src).eval::<Vec<f64>>().unwrap().iter().map(|x| x.to_bits()).collect()
    }

    const GOLDEN: [(&str, [u64; 12]); 11] = [
        ("sin", [0x0, 0x3fb98eaecb8bcb2c, 0x3fdeaee8744b05f0, 0x3feaed548f090cee, 0x3fed18f6ead1b446, 0x3ca1a62633145c07, 0xbfe1689ef5f34f52, 0xbfe03425b78c4db8, 0x3ee4f8b588e1e8a2, 0x3fe91103985da841, 0xbfe49d6e694619b8, 0xbfe9b9dadc41aeb6]),
        ("cos", [0x3ff0000000000000, 0x3fefd712f9a817c1, 0x3fec1528065b7d50, 0x3fe14a280fb5068c, 0xbfdaa22657537205, 0xbff0000000000000, 0xbfead9ac890c6b1f, 0x3feb981dbf665fdf, 0x3feffffffff920c8, 0x3fe3e43a9692e21c, 0x3fe87996529f9d93, 0xbfe307e5980a1558]),
        ("tan", [0x0, 0x3fb9af8877430b80, 0x3fe17b4f5bf3474a, 0x3ff8eb245cbee3a6, 0xc0017af62e0950f8, 0xbca1a62633145c07, 0x3fe4bf5f34be3782, 0xbfe2ca74d62b5d38, 0x3ee4f8b588e6698e, 0x3ff4299ba9c2a138, 0xbfeaf406c2fc78ae, 0x3ff5a0fe5da94891]),
        ("asin", [0x0, 0x3fb9a49276037884, 0x3fe0c152382d7366, 0x0, 0x0, 0x3fc22f5a5204730b, 0x0, 0x0, 0x3ee4f8b588e4e940, 0x3ff1ea93705fa172, 0x3fd380159e14f700, 0x3fde4dc01edf2f8d]),
        ("acos", [0x3ff921fb54442d18, 0x3ff787b22ce3f590, 0x3ff0c152382d7366, 0x3ff921fb54442d18, 0x3ff921fb54442d18, 0x3ff6dc100a039eb7, 0x3ff921fb54442d18, 0x3ff921fb54442d18, 0x3ff921f0d7e968a6, 0x3fdcdd9f8f922e98, 0x3ff441f5ecbeef58, 0x3ff18e8b4c8c6135]),
        ("atan", [0x0, 0x3fd4978fa3269ee2, 0x3ff07c6c6947a6a8, 0x3ff4782cbabc8157, 0x3ff6c020e7493378, 0x3ff79c0667f9f29b, 0x3ff8a7237b339fea, 0x3ff915b19c5070ab, 0x3f0179ec9ca1b551, 0x3ff3fc176b7a8560, 0xbff2a78b84fbf5fc, 0x3ff918074737a9ef]),
        ("exp", [0x3ff0000000000000, 0x3ff03aef2a8100a8, 0x3ff12f4611379bd0, 0x3ff27500916e2887, 0x3ff54a98df86cb6b, 0x3ff9101895a4f158, 0x4010b0e126b3dc0d, 0x41386b40308c47c6, 0x3ff000017f7ac04d, 0x3ff231fb6726feac, 0x3fecf46d99d52b3b, 0x4185c4d43140ce2d]),
        ("log", [0x0, 0x3fb8663f793c46cc, 0x3fd9f323ecbf984c, 0x3fe62e42fefa39ef, 0x3ff193ea7aad030a, 0x3ff6bcbed09f00af, 0x40032ee3b77f374c, 0x401275e2271bba31, 0x3ee4f8aea9af0a25, 0x3fe48a11293d785b, 0x3fe0fae81914a991, 0x40134bba22cd5df5]),
        ("log2", [0x0, 0x3fc199b728cb9d0c, 0x3fe2b803473f7ad1, 0x3ff0000000000000, 0x3ff95c01a39fbd68, 0x400066c7bdd534ad, 0x400bacea7c065d42, 0x401aa20230e1151f, 0x3eee415f0d4c3be2, 0x3feda1c9885a0c3c, 0x3fe87f42b972949d, 0x401bd68510615f84]),
        ("pow", [0x3ff0000000000000, 0x3ff0930586d51442, 0x3ff296fa6a318021, 0x3ff4ad7a2967b268, 0x3ff8064767aade00, 0x3ffb11a8c40a9888, 0x40036d4e8640dafb, 0x4016100de7f7a750, 0x3ff00003e1354354, 0x3ff449f64f58e0e8, 0x3ff3788d12165aaa, 0x4017d5c50fa6516c]),
        ("fmod", [0x0, 0x3fb999999999999a, 0x3fe0000000000000, 0x3fd3333333333334, 0x3fe3333333333334, 0x3fd5dca76eee3590, 0x3fc99999999999b0, 0x3fe333333333336c, 0x3ee4f8b588e368f1, 0x3fc999999999999c, 0x8000000000000000, 0x3fd0624dd2f1aac0]),
    ];

    const U64_SEED42: [u64; 6] = [0x15780b2e0c2ec716, 0x6104d9866d113a7e, 0xae17533239e499a1, 0xecb8ad4703b360a1, 0xfde6dc7fe2ec5e64, 0xc50da53101795238];
    const INT_SEED7: [i64; 10] = [1, 3, 1, 5, 3, 6, 5, 5, 5, 2];
    const FLT_AFTER: [u64; 4] = [0x3fe152e2245ac3ec, 0x3fe76b61e7123e53, 0x3fee0c019551aeb1, 0x3fec2fedefd1598f];

    #[test]
    fn transcendental_goldens_are_bit_exact() {
        let lua = state();
        for ((n, f), (gn, want)) in CASES.iter().zip(GOLDEN.iter()) {
            assert_eq!(n, gn);
            let got = run(&lua, f);
            for (i, (g, w)) in got.iter().zip(want).enumerate() {
                assert_eq!(g, w, "{n} input #{i}: got {g:#x} want {w:#x}");
            }
        }
    }

    #[test]
    fn rng_goldens() {
        let mut g = Rng::new(42);
        let v: Vec<u64> = (0..6).map(|_| g.next_u64()).collect();
        assert_eq!(v, U64_SEED42);
        let lua = state();
        lua.load("math.randomseed(7)").exec().unwrap();
        let i: Vec<i64> = lua.load("local t={} for i=1,10 do t[i]=math.random(1,6) end return t").eval().unwrap();
        assert_eq!(i, INT_SEED7);
        let f: Vec<f64> = lua.load("local t={} for i=1,4 do t[i]=math.random() end return t").eval().unwrap();
        assert_eq!(f.iter().map(|x| x.to_bits()).collect::<Vec<_>>(), FLT_AFTER);
    }

    #[test]
    fn random_argument_semantics() {
        let lua = state();
        let ok: bool = lua
            .load("for i=1,500 do local a,b,c=math.random(),math.random(3),math.random(-2,2) if a<0 or a>=1 or b<1 or b>3 or c<-2 or c>2 or math.type(b)~='integer' then return false end end return true")
            .eval()
            .unwrap();
        assert!(ok);
        assert!(lua.load("math.random(5,1)").exec().is_err());
        assert_eq!(lua.load("return math.random(4,4)").eval::<i64>().unwrap(), 4);
    }

    #[test]
    fn same_seed_same_sequence() {
        let a = crate::sandbox::new_lua();
        let b = crate::sandbox::new_lua();
        install(&a, 99).unwrap();
        install(&b, 99).unwrap();
        let src = "local t={} for i=1,50 do t[i]=math.random(1000) end return t";
        assert_eq!(a.load(src).eval::<Vec<i64>>().unwrap(), b.load(src).eval::<Vec<i64>>().unwrap());
    }
}
