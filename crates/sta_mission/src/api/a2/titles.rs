//! The observed MMM `UI` title and subtitle calls.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, MetaMethod, MultiValue, UserData, UserDataMethods, Value};
use sta_core::Fixed;
use sta_sim::command::Edit;
use sta_sim::present::{Arg, Presentation};

use crate::api::{stub, Shared};

struct Ui {
    ctx: Shared,
    next_title: Rc<RefCell<u32>>,
}

fn text(v: &Value) -> mlua::Result<String> {
    match v {
        Value::String(s) => Ok(s.to_str()?.to_owned()),
        _ => Err(mlua::Error::RuntimeError(
            "UI title text must be a string".into(),
        )),
    }
}

fn number(v: &Value) -> Option<Arg> {
    match v {
        Value::Integer(i) => Some(Arg::Int(*i)),
        Value::Number(n) if n.is_finite() && n.abs() < 2_147_483_648.0 => {
            Some(Arg::Num(Fixed::from_f64(*n)))
        }
        _ => None,
    }
}

fn add(ui: &Ui, args: MultiValue, subtitle: bool) -> mlua::Result<()> {
    let method = if subtitle { "addSubtitle" } else { "addTitle" };
    let id = ui
        .next_title
        .borrow()
        .checked_add(1)
        .ok_or_else(|| mlua::Error::RuntimeError("UI title IDs exhausted".into()))?;
    let args = args.into_vec();
    let event = match args.as_slice() {
        [one] => {
            let text = text(one)?;
            // A plain text title stays until a future, evidenced removal call.
            Presentation::Title {
                id,
                text,
                subtitle,
                seconds: None,
            }
        }
        [one, x, y, duration, Value::Boolean(flag)] => {
            let text = text(one)?;
            let (Some(x), Some(y), Some(seconds)) = (number(x), number(y), number(duration)) else {
                return Err(mlua::Error::RuntimeError(format!(
                    "UI:{method} placement and duration must be finite numbers"
                )));
            };
            // The seam has no placement or final-boolean field. Keep the
            // observed positional arguments verbatim in its generic call;
            // the final boolean's native meaning is still unknown.
            Presentation::Call {
                name: format!("UI.{method}"),
                args: vec![
                    Arg::Int(i64::from(id)),
                    Arg::Text(text),
                    x,
                    y,
                    seconds,
                    Arg::Bool(*flag),
                ],
            }
        }
        _ => {
            return Err(mlua::Error::RuntimeError(format!(
                "UI:{method} expects text or text, x, y, seconds, bool"
            )));
        }
    };
    *ui.next_title.borrow_mut() = id;
    ui.ctx.borrow_mut().edit(Edit::Present(event));
    Ok(())
}

impl UserData for Ui {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("addTitle", |_, ui, args: MultiValue| add(ui, args, false));
        m.add_method("addSubtitle", |_, ui, args: MultiValue| add(ui, args, true));
        m.add_meta_method(MetaMethod::Index, |lua, ui, name: String| {
            ui.ctx.borrow_mut().unsupported(&format!("UI.{name}"));
            stub(lua)
        });
    }
}

pub(super) fn install(lua: &Lua, ctx: &Shared) -> mlua::Result<()> {
    lua.globals().raw_set(
        "UI",
        lua.create_userdata(Ui {
            ctx: ctx.clone(),
            next_title: Rc::new(RefCell::new(0)),
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
        let lua = Lua::new();
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
    fn plain_titles_use_typed_events_and_shared_ids() {
        let (lua, ctx) = runtime();
        lua.load("UI:addTitle('A'); UI:addSubtitle('B'); UI:addTitle('C')")
            .exec()
            .unwrap();
        assert_eq!(
            events(&ctx),
            vec![
                Presentation::Title {
                    id: 1,
                    text: "A".into(),
                    subtitle: false,
                    seconds: None
                },
                Presentation::Title {
                    id: 2,
                    text: "B".into(),
                    subtitle: true,
                    seconds: None
                },
                Presentation::Title {
                    id: 3,
                    text: "C".into(),
                    subtitle: false,
                    seconds: None
                },
            ]
        );
    }

    #[test]
    fn placement_calls_preserve_all_observed_arguments() {
        let (lua, ctx) = runtime();
        lua.load(
            "UI:addTitle('RECOVERY',50,50,7.0,true); UI:addSubtitle('Starbase 176',50,85,7.0,true)",
        )
        .exec()
        .unwrap();
        assert_eq!(
            events(&ctx),
            vec![
                Presentation::Call {
                    name: "UI.addTitle".into(),
                    args: vec![
                        Arg::Int(1),
                        Arg::Text("RECOVERY".into()),
                        Arg::Int(50),
                        Arg::Int(50),
                        Arg::Num(Fixed::from_int(7)),
                        Arg::Bool(true)
                    ]
                },
                Presentation::Call {
                    name: "UI.addSubtitle".into(),
                    args: vec![
                        Arg::Int(2),
                        Arg::Text("Starbase 176".into()),
                        Arg::Int(50),
                        Arg::Int(85),
                        Arg::Num(Fixed::from_int(7)),
                        Arg::Bool(true)
                    ]
                },
            ]
        );
    }

    #[test]
    fn invalid_arguments_are_errors_without_consuming_ids() {
        let (lua, ctx) = runtime();
        for script in [
            "UI:addTitle()",
            "UI:addTitle(7)",
            "UI:addTitle('A',50)",
            "UI:addSubtitle('B',50,85,7,'yes')",
            "UI:addTitle('A',50,85,math.huge,true)",
        ] {
            assert!(lua.load(script).exec().is_err(), "{script}");
        }
        assert!(events(&ctx).is_empty());
        lua.load("UI:addTitle('valid')").exec().unwrap();
        assert_eq!(
            events(&ctx),
            vec![Presentation::Title {
                id: 1,
                text: "valid".into(),
                subtitle: false,
                seconds: None
            }]
        );
        let (other_lua, other_ctx) = runtime();
        other_lua.load("UI:addSubtitle('other')").exec().unwrap();
        assert_eq!(
            events(&other_ctx),
            vec![Presentation::Title {
                id: 1,
                text: "other".into(),
                subtitle: true,
                seconds: None
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
        lua.load("assert(UI:unknown() == true)").exec().unwrap();
        assert_eq!(ctx.borrow().unsupported.get("UI.unknown"), Some(&1));
    }
}
