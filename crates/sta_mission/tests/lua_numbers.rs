//! Number text and `^` are the same on every platform (#1300). Golden strings; the CI runs them
//! on Linux x86-64 and ARM, macOS and Windows.

use sta_mission::sandbox::{new_lua, refill};

fn eval(chunk: &str) -> String {
    let lua = new_lua();
    refill(&lua);
    lua.load(chunk).eval::<String>().unwrap_or_else(|e| panic!("{chunk}: {e}"))
}

#[test]
fn power_is_the_libm_function() {
    let lua = new_lua();
    for (a, b) in [(2.0, 0.5), (10.0, -3.0), (1.1, 100.0), (0.7, 13.37), (3.0, 40.0), (-2.0, 3.0), (2.0, 1e-9)] {
        refill(&lua);
        let got: f64 = lua.load(format!("local a, b = {a:?}, {b:?} return a ^ b")).eval().unwrap();
        assert_eq!(got.to_bits(), libm::pow(a, b).to_bits(), "{a} ^ {b}");
    }
    // Constant folding at compile time goes through the same function.
    let folded: f64 = lua.load("return 1.1 ^ 100").eval().unwrap();
    assert_eq!(folded.to_bits(), libm::pow(1.1, 100.0).to_bits());
}

#[test]
fn non_finite_floats_print_the_same_everywhere() {
    assert_eq!(eval("return tostring(0/0) .. ' ' .. tostring(-(0/0)) .. ' ' .. tostring(1/0) .. ' ' .. tostring(-1/0)"), "nan nan inf -inf");
    assert_eq!(
        eval("local n, i = 0/0, 1/0 return string.format('[%g][%f][%e][%G][%5.1f][%-6g][%+g][% g][%.17g][%a]', n, i, -i, i, i, n, i, i, -n, -i)"),
        "[nan][inf][-inf][INF][  inf][nan   ][+inf][ inf][nan][-inf]"
    );
    assert_eq!(eval("return tostring(0/0 .. '')"), "nan");
}

#[test]
fn finite_floats_print_the_same_everywhere() {
    let got = eval(
        "local out = {} \
         for _, v in ipairs({ 0.1, 0.1 + 0.2, 1 / 3, 100 / 3, 1e15, 1e16, 1e100, 2^53, 2^63, -0.0, 5e-324, 2.5e-308, 123.456, -7.25, 1e-5 }) do \
           out[#out+1] = tostring(v) .. '|' .. string.format('%.17g|%g|%.3f|%e|%a', v, v, v, v, v) end \
         return table.concat(out, ' ')",
    );
    let want = [
        "0.1|0.10000000000000001|0.1|0.100|1.000000e-01|0x1.999999999999ap-4",
        "0.3|0.30000000000000004|0.3|0.300|3.000000e-01|0x1.3333333333334p-2",
        "0.33333333333333|0.33333333333333331|0.333333|0.333|3.333333e-01|0x1.5555555555555p-2",
        "33.333333333333|33.333333333333336|33.3333|33.333|3.333333e+01|0x1.0aaaaaaaaaaabp+5",
        "1e+15|1000000000000000|1e+15|1000000000000000.000|1.000000e+15|0x1.c6bf52634p+49",
        "1e+16|10000000000000000|1e+16|10000000000000000.000|1.000000e+16|0x1.1c37937e08p+53",
        "1e+100|1e+100|1e+100|10000000000000000159028911097599180468360808563945281389781327557747838772170381060813469985856815104.000|1.000000e+100|0x1.249ad2594c37dp+332",
        "9.007199254741e+15|9007199254740992|9.0072e+15|9007199254740992.000|9.007199e+15|0x1p+53",
        "9.2233720368548e+18|9.2233720368547758e+18|9.22337e+18|9223372036854775808.000|9.223372e+18|0x1p+63",
        "-0.0|-0|-0|-0.000|-0.000000e+00|-0x0p+0",
        "4.9406564584125e-324|4.9406564584124654e-324|4.94066e-324|0.000|4.940656e-324|0x1p-1074",
        "2.5e-308|2.4999999999999998e-308|2.5e-308|0.000|2.500000e-308|0x1.1fa182c40c60dp-1022",
        "123.456|123.456|123.456|123.456|1.234560e+02|0x1.edd2f1a9fbe77p+6",
        "-7.25|-7.25|-7.25|-7.250|-7.250000e+00|-0x1.dp+2",
        "1e-05|1.0000000000000001e-05|1e-05|0.000|1.000000e-05|0x1.4f8b588e368f1p-17",
    ];
    let got: Vec<&str> = got.split(' ').collect();
    let wrong: Vec<String> = got.iter().zip(want).filter(|(g, w)| *g != w).map(|(g, w)| format!("got {g}, want {w}")).collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
    assert_eq!(got.len(), want.len());
}
