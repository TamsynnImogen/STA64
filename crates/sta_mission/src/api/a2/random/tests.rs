use crate::runtime::MissionRuntime;

fn runtime(seed: u64, source: &str) -> MissionRuntime {
    let rt = MissionRuntime::from_source("random.lua", source, seed);
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    rt
}

#[test]
fn unsigned_bounds_and_result_type_are_strictly_integral() {
    let mut rt = runtime(
        41,
        r#"
        a = A2.RandomGetUnsigned(1)
        b = A2.RandomGetUnsigned(17.0)
        c = A2.RandomGetUnsigned(4294967295)
        "#,
    );
    let (a, b, c, a_integer, b_integer, c_integer): (i64, i64, i64, bool, bool, bool) = rt
        .eval("return a, b, c, math.type(a)=='integer', math.type(b)=='integer', math.type(c)=='integer'")
        .unwrap();
    assert_eq!(a, 0, "upper bound one has only result zero");
    assert!((0..17).contains(&b));
    assert!((0..4_294_967_295).contains(&c));
    assert!((a_integer, b_integer, c_integer) == (true, true, true));
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn same_seed_repeats_and_different_seed_changes_the_stream() {
    fn draws(seed: u64) -> Vec<i64> {
        runtime(seed, "")
            .eval(
                r#"
            local result = {}
            for i = 1, 12 do result[i] = A2.RandomGetUnsigned(4294967295) end
            return result
            "#,
            )
            .unwrap()
    }

    let first = draws(0x57a2);
    assert_eq!(first, draws(0x57a2));
    assert_ne!(first, draws(0x57a3));
}

#[test]
fn a2_and_math_draws_share_the_reseedable_stream_including_upper_one() {
    let baseline = runtime(73, "");
    let expected: (i64, i64, i64, i64) = baseline
        .eval(
            "math.randomseed(991); return math.random(1,1000), math.random(1,1000), math.random(1,1000), math.random(1,1000)",
        )
        .unwrap();

    let mut interleaved = runtime(73, "");
    let (first, one, third, reset_one, after_reset): (i64, i64, i64, i64, i64) = interleaved
        .eval(
            r#"
            math.randomseed(991)
            local first = math.random(1,1000)
            local one = A2.RandomGetUnsigned(1)
            local third = math.random(1,1000)
            math.randomseed(991)
            local reset_one = A2.RandomGetUnsigned(1)
            local after_reset = math.random(1,1000)
            return first, one, third, reset_one, after_reset
            "#,
        )
        .unwrap();
    assert_eq!(first, expected.0);
    assert_eq!(one, 0);
    assert_eq!(third, expected.2, "upper-one call must consume one draw");
    assert_eq!(reset_one, 0);
    assert_eq!(
        after_reset, expected.1,
        "randomseed resets the shared stream"
    );
    assert!(interleaved.take_commands().is_empty());
    assert!(interleaved.unsupported().is_empty());
}

#[test]
fn rejected_arguments_do_not_consume_randomness() {
    let baseline = runtime(101, "");
    let expected: (i64, i64) = baseline
        .eval("return A2.RandomGetUnsigned(1000), math.random(1,1000)")
        .unwrap();

    let mut rt = runtime(101, "");
    let (all_rejected, actual_a, actual_math): (bool, i64, i64) = rt
        .eval(
            r#"
            local bad = {
                function() A2.RandomGetUnsigned() end,
                function() A2.RandomGetUnsigned(nil) end,
                function() A2.RandomGetUnsigned(10, 20) end,
                function() A2.RandomGetUnsigned(0) end,
                function() A2.RandomGetUnsigned(-1) end,
                function() A2.RandomGetUnsigned(1.5) end,
                function() A2.RandomGetUnsigned(4294967296) end,
                function() A2.RandomGetUnsigned(math.huge) end,
                function() A2.RandomGetUnsigned(0/0) end,
                function() A2.RandomGetUnsigned('10') end,
                function() A2.RandomGetUnsigned(true) end,
                function() A2.RandomGetUnsigned({}) end
            }
            for _, call in ipairs(bad) do
                if pcall(call) then return false, 0, 0 end
            end
            return true, A2.RandomGetUnsigned(1000), math.random(1,1000)
            "#,
        )
        .unwrap();
    assert!(all_rejected);
    assert_eq!(
        (actual_a, actual_math),
        expected,
        "invalid calls must fail before drawing"
    );
    assert!(rt.errors().is_empty(), "{:?}", rt.errors());
    assert!(rt.take_commands().is_empty());
    assert!(rt.unsupported().is_empty());
}

#[test]
fn a2_keeps_the_installed_random_function_after_global_replacement() {
    let baseline = runtime(303, "");
    let expected: i64 = baseline
        .eval("return A2.RandomGetUnsigned(1000000)")
        .unwrap();

    let mut replaced = runtime(
        303,
        "math.random = function() return 999 end; replaced_value = A2.RandomGetUnsigned(1000000)",
    );
    let (actual, global): (i64, i64) = replaced
        .eval("return replaced_value, math.random(1,1000)")
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(global, 999);
    assert!(replaced.errors().is_empty(), "{:?}", replaced.errors());
    assert!(replaced.take_commands().is_empty());
    assert!(replaced.unsupported().is_empty());
}
