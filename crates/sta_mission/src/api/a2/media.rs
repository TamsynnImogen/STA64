//! The observed MMM `Media` calls, queued as presentation commands.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, MetaMethod, MultiValue, UserData, UserDataMethods, Value};
use sta_sim::command::Edit;
use sta_sim::present::Presentation;

use crate::api::{stub, Shared};

#[derive(Default)]
struct MediaState {
    next_sound: u32,
    sounds: Vec<u32>,
}

struct Media {
    ctx: Shared,
    state: Rc<RefCell<MediaState>>,
}

fn no_args(name: &str, args: &MultiValue) -> mlua::Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(mlua::Error::RuntimeError(format!(
            "Media:{name} takes no arguments"
        )))
    }
}

fn one_name(name: &str, args: &MultiValue) -> mlua::Result<String> {
    match (args.len(), args.front()) {
        (1, Some(Value::String(s))) => Ok(s.to_str()?.to_owned()),
        _ => Err(mlua::Error::RuntimeError(format!(
            "Media:{name} requires exactly one string name"
        ))),
    }
}

impl UserData for Media {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("playSound", |_, media, args: MultiValue| {
            let name = one_name("playSound", &args)?;
            let mut state = media.state.borrow_mut();
            let id = state
                .next_sound
                .checked_add(1)
                .ok_or_else(|| mlua::Error::RuntimeError("Media sound IDs exhausted".into()))?;
            state.next_sound = id;
            state.sounds.push(id);
            media
                .ctx
                .borrow_mut()
                .edit(Edit::Present(Presentation::Sound { id, name, at: None }));
            Ok(id)
        });
        m.add_method("stopSounds", |_, media, args: MultiValue| {
            no_args("stopSounds", &args)?;
            // Arrival order is stable and matches the order of playSound calls.
            for id in media.state.borrow_mut().sounds.drain(..) {
                media
                    .ctx
                    .borrow_mut()
                    .edit(Edit::Present(Presentation::StopSound { id }));
            }
            Ok(())
        });
        m.add_method("playMusic", |_, media, args: MultiValue| {
            let name = one_name("playMusic", &args)?;
            // The one-argument bridge form loops by default.
            media
                .ctx
                .borrow_mut()
                .edit(Edit::Present(Presentation::Music { name, looped: true }));
            Ok(())
        });
        m.add_method("stopMusic", |_, media, args: MultiValue| {
            no_args("stopMusic", &args)?;
            media
                .ctx
                .borrow_mut()
                .edit(Edit::Present(Presentation::StopMusic));
            Ok(())
        });
        m.add_method("playMovie", |_, media, args: MultiValue| {
            let name = one_name("playMovie", &args)?;
            // The one-argument bridge form is a skippable full-screen movie.
            media
                .ctx
                .borrow_mut()
                .edit(Edit::Present(Presentation::Movie {
                    name,
                    in_panel: false,
                    skippable: true,
                }));
            Ok(())
        });
        m.add_method("stopMovie", |_, media, args: MultiValue| {
            no_args("stopMovie", &args)?;
            media
                .ctx
                .borrow_mut()
                .edit(Edit::Present(Presentation::StopMovie));
            Ok(())
        });
        m.add_meta_method(MetaMethod::Index, |lua, media, name: String| {
            media.ctx.borrow_mut().unsupported(&format!("Media.{name}"));
            stub(lua)
        });
    }
}

pub(super) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    lua.globals().raw_set(
        "Media",
        lua.create_userdata(Media {
            ctx: ctx.clone(),
            state: Rc::default(),
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{Ctx, MissionConfig};
    use sta_core::PlayerId;
    use sta_sim::command::CommandKind;

    fn runtime() -> (Lua, Shared) {
        let lua = crate::sandbox::new_lua();
        let ctx = Rc::new(RefCell::new(Ctx::new(MissionConfig::default(), 1)));
        install(&lua, &ctx).unwrap();
        (lua, ctx)
    }

    fn events(ctx: &Shared) -> Vec<Presentation> {
        ctx.borrow()
            .commands
            .iter()
            .map(|c| {
                assert_eq!(c.player, PlayerId::SCRIPT);
                match &c.kind {
                    CommandKind::Edit(Edit::Present(p)) => p.clone(),
                    other => panic!("{other:?}"),
                }
            })
            .collect()
    }

    #[test]
    fn emits_typed_commands_in_order_and_clears_sounds() {
        let (lua, ctx) = runtime();
        lua.load("a=Media:playSound('a.wav'); b=Media:playSound('b.wav'); Media:stopSounds(); Media:stopSounds(); c=Media:playSound('c.wav'); Media:playMusic('m.wav'); Media:stopMusic(); Media:playMovie('v.bik'); Media:stopMovie()").exec().unwrap();
        assert_eq!(lua.globals().get::<u32>("a").unwrap(), 1);
        assert_eq!(lua.globals().get::<u32>("b").unwrap(), 2);
        assert_eq!(lua.globals().get::<u32>("c").unwrap(), 3);
        assert_eq!(
            events(&ctx),
            vec![
                Presentation::Sound {
                    id: 1,
                    name: "a.wav".into(),
                    at: None
                },
                Presentation::Sound {
                    id: 2,
                    name: "b.wav".into(),
                    at: None
                },
                Presentation::StopSound { id: 1 },
                Presentation::StopSound { id: 2 },
                Presentation::Sound {
                    id: 3,
                    name: "c.wav".into(),
                    at: None
                },
                Presentation::Music {
                    name: "m.wav".into(),
                    looped: true
                },
                Presentation::StopMusic,
                Presentation::Movie {
                    name: "v.bik".into(),
                    in_panel: false,
                    skippable: true
                },
                Presentation::StopMovie,
            ]
        );
    }

    #[test]
    fn invalid_arguments_do_not_queue_commands_and_state_is_per_runtime() {
        let (lua, ctx) = runtime();
        for script in [
            "Media:playSound()",
            "Media:playSound('x', true)",
            "Media:playMusic(1)",
            "Media:playMovie('v', false)",
            "Media:stopSounds(1)",
            "Media:stopMovie('v')",
        ] {
            assert!(lua.load(script).exec().is_err(), "{script}");
        }
        assert!(events(&ctx).is_empty());
        let (other_lua, other_ctx) = runtime();
        other_lua
            .load("id=Media:playSound('other.wav')")
            .exec()
            .unwrap();
        assert_eq!(other_lua.globals().get::<u32>("id").unwrap(), 1);
        assert_eq!(
            events(&other_ctx),
            vec![Presentation::Sound {
                id: 1,
                name: "other.wav".into(),
                at: None
            }]
        );
    }

    #[test]
    fn unknown_method_uses_runtime_stub_and_records_gap() {
        let (lua, ctx) = runtime();
        lua.load(
            "__stub = function() return setmetatable({}, {__call=function() return true end}) end",
        )
        .exec()
        .unwrap();
        lua.load("assert(Media:unknown() == true)").exec().unwrap();
        assert_eq!(ctx.borrow().unsupported.get("Media.unknown"), Some(&1));
    }
}
