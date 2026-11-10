//! The manual Send boundary requires Lua handles to stay inside the runtime.
//! Send alone is insufficient: a custom FromLua/IntoLua implementation can
//! retain a Lua clone as a side effect. Only these sealed standard conversions
//! may inspect the VM. Do not admit Value, Function, Table or userdata here.

mod sealed {
    pub trait Result {}
    pub trait Args {}
}

/// Owned inspection results: standard scalars, String, Option, Vec and tuples
/// of up to eight sealed values. Custom Lua conversions are intentionally excluded.
pub trait EvalResult: mlua::FromLuaMulti + Send + sealed::Result {}

/// Inspection arguments with standard conversions, including borrowed str.
/// No caller-defined conversion receives access to the runtime's Lua state.
pub trait EvalArgs: mlua::IntoLuaMulti + Send + sealed::Args {}

// Changes to mlua features must not let errors carry thread-affine state out
// through the public Result either. This assertion is checked in every build.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<mlua::Error>();
};

macro_rules! scalars {
    ($($ty:ty),+ $(,)?) => { $(
        impl sealed::Result for $ty {}
        impl EvalResult for $ty {}
        impl sealed::Args for $ty {}
        impl EvalArgs for $ty {}
    )+ };
}
scalars!(
    (),
    bool,
    i8,
    i16,
    i32,
    i64,
    isize,
    u8,
    u16,
    u32,
    u64,
    usize,
    f32,
    f64,
    String
);

impl sealed::Args for &str {}
impl EvalArgs for &str {}

impl<T: EvalResult + mlua::FromLua> sealed::Result for Option<T> {}
impl<T: EvalResult + mlua::FromLua> EvalResult for Option<T> {}
impl<T: EvalResult + mlua::FromLua> sealed::Result for Vec<T> {}
impl<T: EvalResult + mlua::FromLua> EvalResult for Vec<T> {}
impl<T: EvalArgs + mlua::IntoLua> sealed::Args for Option<T> {}
impl<T: EvalArgs + mlua::IntoLua> EvalArgs for Option<T> {}
impl<T: EvalArgs + mlua::IntoLua> sealed::Args for Vec<T> {}
impl<T: EvalArgs + mlua::IntoLua> EvalArgs for Vec<T> {}

macro_rules! tuple {
    ($($t:ident),+) => {
        impl<$($t: EvalResult + mlua::FromLua),+> sealed::Result for ($($t,)+) {}
        impl<$($t: EvalResult + mlua::FromLua),+> EvalResult for ($($t,)+) {}
        impl<$($t: EvalArgs + mlua::IntoLua),+> sealed::Args for ($($t,)+) {}
        impl<$($t: EvalArgs + mlua::IntoLua),+> EvalArgs for ($($t,)+) {}
    };
}
tuple!(A);
tuple!(A, B);
tuple!(A, B, C);
tuple!(A, B, C, D);
tuple!(A, B, C, D, E);
tuple!(A, B, C, D, E, F);
tuple!(A, B, C, D, E, F, G);
tuple!(A, B, C, D, E, F, G, H);

#[cfg(test)]
mod tests {
    use crate::runtime::MissionRuntime;

    #[test]
    fn inspection_copies_owned_results_and_accepts_sealed_dynamic_arguments() {
        let rt = MissionRuntime::from_source("inspection.lua", "", 1);
        let pair: (String, u32) = rt
            .eval_with("local name, n = ...; return name, n + 1", ("test", 6u32))
            .unwrap();
        assert_eq!(pair, ("test".to_owned(), 7));
        let values: Vec<u32> = rt.eval("return {2, 4, 8}").unwrap();
        assert_eq!(values, [2, 4, 8]);
        assert_eq!(rt.eval::<Option<String>>("return nil").unwrap(), None);
        assert!(rt.eval::<String>("return function() end").is_err());
        assert_eq!(
            rt.eval_with::<String, _>("return ...", "safe 'quoted' argument")
                .unwrap(),
            "safe 'quoted' argument"
        );
    }

    #[test]
    fn owned_inspection_data_can_stay_while_the_runtime_moves_to_another_thread() {
        let rt = MissionRuntime::from_source("inspection.lua", "", 1);
        let data: Vec<u32> = rt.eval("return {2, 4, 8}").unwrap();
        let thread = std::thread::spawn(move || rt.eval::<u32>("return 9").unwrap());
        assert_eq!(data, [2, 4, 8]);
        assert_eq!(thread.join().unwrap(), 9);
    }
}
