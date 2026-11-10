//! The converted missions' `Cineractive` singleton and named `Path` handles.
//!
//! `lookFrom` retains a source; `lookAt` completes a simple entity/vector
//! pair with an instantaneous typed shot. Other calls stay generic, including
//! paths and their extra parameters. No meaning is assigned to the observed
//! `lookFrom(Path(name), number)` parameter yet.
//! One-argument fades use black. Single-vector offsets use `relative=false`;
//! these are bridge defaults, not a claim about unobserved native overloads.

use mlua::{Lua, MetaMethod, UserData, UserDataFields, UserDataMethods, Value, Variadic};
use sta_core::{Fixed, Vec3};
use sta_sim::command::Edit;
use sta_sim::present::{Arg, Presentation};
#[cfg(test)]
use sta_sim::present::Spot;

use crate::api::Shared;
use crate::api::entity::Entity;
use crate::api::camera_intent::{self, ArgRef, Intent, SpotRef};

struct PathRef {
    name: String,
    ctx: Shared,
}

impl UserData for PathRef {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("name", |_, p| Ok(p.name.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Index, |lua, p, name: String| {
            p.ctx.borrow_mut().unsupported(&format!("Path.{name}"));
            crate::api::stub(lua)
        });
    }
}

fn fixed(n: f64) -> mlua::Result<Fixed> {
    if !n.is_finite() || !(-2147483648.0..2147483648.0).contains(&n) {
        return Err(mlua::Error::runtime(
            "presentation number must be finite and fit 32.32 fixed point",
        ));
    }
    Ok(Fixed::from_f64(n))
}

fn position(t: &mlua::Table) -> mlua::Result<Vec3> {
    Ok(Vec3 {
        x: fixed(t.get("x")?)?,
        y: fixed(t.get("y")?)?,
        z: fixed(t.get("z")?)?,
    })
}

fn spot(v: &Value) -> mlua::Result<SpotRef> {
    match v {
        Value::UserData(u) if u.is::<Entity>() => {
            let key = u.borrow::<Entity>()?.key.clone();
            if key.id().is_none() && !key.pending() {
                return Err(mlua::Error::runtime("camera entity spawn failed"));
            }
            Ok(SpotRef::Entity(key))
        }
        Value::UserData(u) if u.is::<PathRef>() => Ok(SpotRef::Path(u.borrow::<PathRef>()?.name.clone())),
        Value::Table(t) => Ok(SpotRef::Position(position(t)?)),
        _ => Err(mlua::Error::runtime("camera spot must be an entity, Vector or Path")),
    }
}

fn arg(v: &Value) -> mlua::Result<ArgRef> {
    match v {
        Value::Boolean(b) => Ok(ArgRef::Ready(Arg::Bool(*b))),
        Value::Integer(n) => Ok(ArgRef::Ready(Arg::Int(*n))),
        Value::Number(n) => Ok(ArgRef::Ready(Arg::Num(fixed(*n)?))),
        Value::String(s) => Ok(ArgRef::Ready(Arg::Text(s.to_str()?.to_owned()))),
        _ => spot(v).map(ArgRef::Spot),
    }
}

struct Cineractive {
    ctx: Shared,
    active: bool,
    skippable: bool,
    from: Option<SpotRef>,
    from_typed: bool,
}

/// Copy the existing singleton's state without calling Lua or exporting a VM handle.
pub(super) fn active(lua: &Lua) -> mlua::Result<bool> {
    let singleton = lua.globals().raw_get::<mlua::AnyUserData>("Cineractive")?;
    let active = singleton.borrow::<Cineractive>()?.active;
    Ok(active)
}

impl Cineractive {
    fn emit(&self, p: Presentation) {
        self.ctx.borrow_mut().edit(Edit::Present(p));
    }

    fn mode(&self) {
        self.emit(Presentation::Cineractive {
            on: self.active,
            skippable: self.skippable,
        });
    }

    fn look(&mut self, from: bool, values: Variadic<Value>) -> mlua::Result<()> {
        let Some(first) = values.first() else {
            return Err(mlua::Error::runtime("camera spot is required"));
        };
        let s = spot(first)?;
        // Validate every extra before mutating state or emitting anything.
        let mut args = vec![ArgRef::Spot(s.clone())];
        for v in values.iter().skip(1) {
            args.push(arg(v)?);
        }
        let simple = args.len() == 1 && !matches!(&s, SpotRef::Path(_));
        if from {
            self.from = Some(s.clone());
            self.from_typed = simple;
            camera_intent::clear(&mut self.ctx.borrow_mut());
        } else if simple
            && self.from_typed
            && let Some(source) = &self.from
        {
            // Complete the pair only here: a new source must not produce a
            // transient shot toward an earlier scene's target. Parameterized
            // paths remain generic instead of adding an approximate Shot.
            return camera_intent::submit(&mut self.ctx.borrow_mut(), Intent::Shot {
                from: source.clone(),
                at: s,
            }).map_err(mlua::Error::runtime);
        }
        camera_intent::submit(&mut self.ctx.borrow_mut(), Intent::Call {
            name: if from {
                "Cineractive.lookFrom"
            } else {
                "Cineractive.lookAt"
            },
            args,
        }).map_err(mlua::Error::runtime)
    }
}

impl UserData for Cineractive {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("active", |_, c| Ok(c.active));
        f.add_field_method_get("skippable", |_, c| Ok(c.skippable));
        f.add_field_method_set("active", |_, c, on: bool| {
            c.active = on;
            if !on {
                camera_intent::clear(&mut c.ctx.borrow_mut());
                // A subsequent lookAt must not resurrect a canceled pending source.
                // Preserve the previous behavior of already resolved sources.
                if c.from.as_ref().is_some_and(SpotRef::unresolved) {
                    c.from = None;
                    c.from_typed = false;
                }
            }
            c.mode();
            Ok(())
        });
        f.add_field_method_set("skippable", |_, c, on: bool| {
            c.skippable = on;
            c.mode();
            Ok(())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::Index, |lua, c, name: String| {
            c.ctx.borrow_mut().unsupported(&format!("Cineractive.{name}"));
            crate::api::stub(lua)
        });
        m.add_method_mut("lookFrom", |_, c, args: Variadic<Value>| c.look(true, args));
        m.add_method_mut("lookAt", |_, c, args: Variadic<Value>| c.look(false, args));
        m.add_method("setCameraOffset", |_, c, args: Variadic<Value>| {
            let [Value::Table(t)] = args.as_slice() else {
                return Err(mlua::Error::runtime(
                    "setCameraOffset expects one Vector; other overloads are not implemented",
                ));
            };
            c.emit(Presentation::CameraOffset {
                offset: position(t)?,
                relative: false,
            });
            Ok(())
        });
        for (name, out) in [("fadeIn", false), ("fadeOut", true)] {
            m.add_method(name, move |_, c, args: Variadic<f64>| {
                let [n] = args.as_slice() else {
                    return Err(mlua::Error::runtime(
                        "fade expects one duration; colour overloads are not implemented",
                    ));
                };
                let seconds = fixed(*n)?;
                if seconds < Fixed::ZERO {
                    return Err(mlua::Error::runtime("fade duration must be nonnegative"));
                }
                c.emit(Presentation::Fade {
                    out,
                    seconds,
                    rgb: [0; 3],
                });
                Ok(())
            });
        }
    }
}

pub(super) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    lua.globals().raw_set(
        "Cineractive",
        lua.create_userdata(Cineractive {
            ctx: ctx.clone(),
            active: false,
            skippable: false,
            from: None,
            from_typed: false,
        })?,
    )?;
    let path = lua.create_table()?;
    let mt = lua.create_table()?;
    let c = ctx.clone();
    mt.set(
        "__call",
        lua.create_function(move |_, (_class, name): (Value, String)| {
            if name.is_empty() {
                return Err(mlua::Error::runtime("Path requires a name"));
            }
            Ok(PathRef { name, ctx: c.clone() })
        })?,
    )?;
    let c = ctx.clone();
    mt.set(
        "__index",
        lua.create_function(move |lua, (_class, name): (Value, String)| {
            c.borrow_mut().unsupported(&format!("Path.{name}"));
            crate::api::stub(lua)
        })?,
    )?;
    path.set_metatable(Some(mt))?;
    lua.globals().raw_set("Path", path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::MissionRuntime;
    use sta_core::PlayerId;
    use sta_sim::command::CommandKind;

    fn presentations(rt: &mut MissionRuntime) -> Vec<Presentation> {
        rt.take_commands()
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                assert_eq!(c.player, PlayerId::SCRIPT);
                assert_eq!(c.seq, i as u32 + 1);
                match c.kind {
                    CommandKind::Edit(Edit::Present(p)) => p,
                    other => panic!("{other:?}"),
                }
            })
            .collect()
    }

    #[test]
    fn vectors_mode_offsets_and_fades_become_script_commands() {
        let mut rt = MissionRuntime::from_source(
            "T.lua",
            r#"
            Cineractive.active = true; Cineractive.skippable = true
            Cineractive:lookFrom(Vector(1, 2, 3)); Cineractive:lookAt(Vector(4, 5, 6))
            Cineractive:setCameraOffset(Vector(-10, 20, -30)); Cineractive:fadeIn(3); Cineractive:fadeOut(0.5)
            ACTIVE, SKIP = Cineractive.active, Cineractive.skippable
        "#,
            1,
        );
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        assert_eq!(rt.eval::<(bool, bool)>("return ACTIVE, SKIP").unwrap(), (true, true));
        let p = presentations(&mut rt);
        assert_eq!(
            p[0],
            Presentation::Cineractive {
                on: true,
                skippable: false
            }
        );
        assert_eq!(
            p[1],
            Presentation::Cineractive {
                on: true,
                skippable: true
            }
        );
        assert_eq!(
            p[3],
            Presentation::Shot {
                from: Spot::Position(Vec3::from_f32([1., 2., 3.])),
                at: Spot::Position(Vec3::from_f32([4., 5., 6.])),
                seconds: Fixed::ZERO
            }
        );
        assert_eq!(
            p[4],
            Presentation::CameraOffset {
                offset: Vec3::from_f32([-10., 20., -30.]),
                relative: false
            }
        );
        assert_eq!(
            p[5],
            Presentation::Fade {
                out: false,
                seconds: Fixed::from_int(3),
                rgb: [0; 3]
            }
        );
        assert_eq!(
            p[6],
            Presentation::Fade {
                out: true,
                seconds: Fixed::from_f64(0.5),
                rgb: [0; 3]
            }
        );
    }

    #[test]
    fn named_paths_keep_extra_parameters_and_state_is_per_runtime() {
        let mut rt = MissionRuntime::from_source(
            "T.lua",
            "Cineractive:lookFrom(Path('opening'), 0.14); Cineractive:lookAt(Vector(1,2,3))",
            1,
        );
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        let p = presentations(&mut rt);
        assert_eq!(
            p[0],
            Presentation::Call {
                name: "Cineractive.lookFrom".into(),
                args: vec![Arg::Text("opening".into()), Arg::Num(Fixed::from_f64(0.14))]
            }
        );
        assert!(matches!(&p[1], Presentation::Call { name, .. } if name == "Cineractive.lookAt"));
        let mut fresh = MissionRuntime::from_source("T.lua", "Cineractive:lookAt(Vector(1,2,3))", 1);
        assert!(matches!(
            presentations(&mut fresh).as_slice(),
            [Presentation::Call { .. }]
        ));
    }

    #[test]
    fn later_camera_pairs_do_not_shoot_toward_a_previous_target_or_approximate_paths() {
        let mut rt = MissionRuntime::from_source(
            "T.lua",
            r#"
            Cineractive:lookFrom(Vector(1,0,0)); Cineractive:lookAt(Vector(2,0,0))
            Cineractive:lookFrom(Vector(3,0,0)); Cineractive:lookAt(Vector(4,0,0))
            Cineractive:lookFrom(Path('next'), 0.14); Cineractive:lookAt(Vector(5,0,0))
        "#,
            1,
        );
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        let p = presentations(&mut rt);
        assert_eq!(p.len(), 6);
        assert!(matches!(&p[2], Presentation::Call { name, .. } if name == "Cineractive.lookFrom"));
        let shots: Vec<_> = p
            .iter()
            .filter_map(|p| match p {
                Presentation::Shot { from, at, .. } => Some((from.clone(), at.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            shots,
            vec![
                (
                    Spot::Position(Vec3::from_f32([1., 0., 0.])),
                    Spot::Position(Vec3::from_f32([2., 0., 0.]))
                ),
                (
                    Spot::Position(Vec3::from_f32([3., 0., 0.])),
                    Spot::Position(Vec3::from_f32([4., 0., 0.]))
                ),
            ]
        );
        assert!(matches!(&p[4], Presentation::Call { name, .. } if name == "Cineractive.lookFrom"));
        assert!(matches!(&p[5], Presentation::Call { name, .. } if name == "Cineractive.lookAt"));
    }

    #[test]
    fn malformed_arguments_do_not_emit_or_change_camera_state() {
        let mut rt = MissionRuntime::from_source(
            "T.lua",
            r#"
            assert(not pcall(function() Cineractive:fadeIn(-1) end))
            assert(not pcall(function() Cineractive:fadeIn(0/0) end))
            assert(not pcall(function() Cineractive:fadeIn(1, 2) end))
            assert(not pcall(function() Cineractive:setCameraOffset(Vector(1,2,3), true) end))
            assert(not pcall(function() Cineractive:lookFrom(Vector(1,2,3), function() end) end))
            assert(not pcall(function() Cineractive:lookAt(Vector(math.huge,0,0)) end))
            Cineractive:lookAt(Vector(4,5,6))
        "#,
            1,
        );
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        assert!(matches!(presentations(&mut rt).as_slice(), [Presentation::Call { .. }]));
    }

    #[test]
    fn unimplemented_path_api_retains_stubs_instead_of_fabricating_geometry() {
        let rt = MissionRuntime::from_source(
            "T.lua",
            r#"
            local path = Path('opening')
            assert(path.name == 'opening')
            path:remove()
            Path.exists('opening')
        "#,
            1,
        );
        assert!(rt.errors().is_empty(), "{:?}", rt.errors());
        assert_eq!(rt.unsupported(), [("Path.exists".into(), 1), ("Path.remove".into(), 1)]);
    }
}
