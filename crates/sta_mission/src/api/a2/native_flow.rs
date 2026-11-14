//! Exact A2 presentation calls that take the calling script (`script *`):
//! `FadeIn`, `FadeOut`, `CineractiveStart`, `AudioMessage`, `PlayMusic`,
//! `PlayCinematicMovie` and `WaitForEnterKey`, plus `StopAudioMessage`.
//!
//! # The script argument and step waits
//!
//! An Armada II mission script is a state machine whose current step ends when
//! a reference count, held on the script object, returns to zero. Each of these
//! engine calls takes the calling script and, unless the trailing "don't wait"
//! flag is true, adds one reference to it that the engine releases when the
//! operation ends (read from the executable, static). The Lua binding keeps that
//! shape: the first argument is the script object, any Lua table the mission
//! (or its converter) uses as the script's identity, and a waiting call records
//! one wait on it. `A2.ScriptWaits(script)` returns how many are still
//! outstanding (the step's reference count), so a converted step machine can
//! advance when it reads zero. Nothing here blocks the Lua call itself.
//!
//! Release rules: a fade after `delay + duration` seconds of mission time (the
//! engine's own fade timer), a sound or music message when the client reports
//! it finished (`A2.AudioMessageDone(id)`, as for `IsAudioMessageDone`) or the
//! mission stops it, a movie on `A2.MovieDone()` or when the mission stops it,
//! an Enter wait on `A2.EnterKeyDone(id)` with the id of the queued
//! `WaitForKey` event, and a cineractive start once its shots are done or the player
//! skipped (the runtime calls `A2.CineractiveStartDone()` then, UI-36; static: the
//! engine's cineractive update releases it, not `CineractiveEnd`).
//! The three host reports are not DLL calls; no client consumer exists yet, so
//! until a host reports them those waits stay outstanding.
//!
//! Evidence grade: argument roles, the "don't wait" inversion (flag true means
//! no reference is taken), the fade fields (duration, delay, white-or-black),
//! the shared sound-id counter starting at 1, and the "mission over" early
//! return are read from the executable, static. Whether a
//! stopped sound releases its wait, and the movie `int` (always 1 in the stock
//! scripts, unused by the video start) are not measured.

use std::cell::RefCell;

use mlua::{Lua, MultiValue, ObjectLike, Table, Value};
use sta_core::{Fixed, TICK_HZ};
use sta_sim::command::{CommandKind, Edit};
use sta_sim::present::{Arg, Presentation};

use super::native_camera::{arity, boolean, submit, text};

use crate::api::Shared;

enum Kind {
    /// A fade, finished at this mission tick.
    Fade(u64),
    Sound(u32),
    Movie,
    Key(u32),
    Cineractive,
}

struct Wait {
    /// Held so the table (and its address) stay alive while it is waiting.
    script: Table,
    kind: Kind,
}

/// Something the client would play or ask for, kept for [`HostPending`]:
/// (kind, id, name, mission tick it started).
struct Started {
    kind: &'static str,
    id: i64,
    name: String,
    tick: u64,
}

struct Flow {
    waits: Vec<Wait>,
    started: Vec<Started>,
    /// `AudioOverlay`'s id counter: starts at 1, shared by voice and music.
    next_sound: u32,
    next_key: u32,
}

fn flow<R>(lua: &Lua, f: impl FnOnce(&mut Flow) -> R) -> R {
    if lua.app_data_ref::<RefCell<Flow>>().is_none() {
        lua.set_app_data(RefCell::new(Flow { waits: Vec::new(), started: Vec::new(), next_sound: 1, next_key: 1 }));
    }
    let cell = lua.app_data_ref::<RefCell<Flow>>().expect("flow state");
    let mut state = cell.borrow_mut();
    f(&mut state)
}

fn script_arg(name: &str, value: &Value) -> mlua::Result<Table> {
    match value {
        Value::Table(t) => Ok(t.clone()),
        _ => Err(mlua::Error::runtime(format!(
            "A2.{name} requires the calling script object (a table) as its first argument"
        ))),
    }
}

/// A finite, non-negative Lua number.
fn seconds(name: &str, what: &str, value: &Value) -> mlua::Result<f64> {
    let n = match value {
        Value::Integer(n) => *n as f64,
        Value::Number(n) if n.is_finite() => *n,
        _ => return Err(mlua::Error::runtime(format!("A2.{name} {what} must be a finite number"))),
    };
    if !(0.0..2_147_483_648.0).contains(&n) {
        return Err(mlua::Error::runtime(format!("A2.{name} {what} must be within 0..2^31")));
    }
    Ok(n)
}

fn finite(name: &str, what: &str, value: &Value) -> mlua::Result<f64> {
    match value {
        Value::Integer(n) => Ok(*n as f64),
        Value::Number(n) if n.is_finite() => Ok(*n),
        _ => Err(mlua::Error::runtime(format!("A2.{name} {what} must be a finite number"))),
    }
}

fn int32(name: &str, what: &str, value: &Value) -> mlua::Result<i64> {
    match value {
        Value::Integer(n) if i32::try_from(*n).is_ok() => Ok(*n),
        _ => Err(mlua::Error::runtime(format!("A2.{name} {what} must be a signed 32-bit integer"))),
    }
}

fn flag(name: &str, value: &Value) -> mlua::Result<bool> {
    match boolean(name, value)? {
        Arg::Bool(b) => Ok(b),
        _ => unreachable!("boolean() returns Bool"),
    }
}

fn present(ctx: &Shared, name: &str, event: Presentation) -> mlua::Result<()> {
    ctx.borrow_mut()
        .try_push(sta_core::PlayerId::SCRIPT, CommandKind::Edit(Edit::Present(event)))
        .map(|_| ())
        .map_err(|reason| mlua::Error::runtime(format!("A2.{name}: {reason}")))
}

fn log_start(lua: &Lua, ctx: &Shared, kind: &'static str, id: i64, name: &str) {
    let tick = ctx.borrow().view.tick;
    flow(lua, |f| {
        if kind == "movie" {
            // One movie plays at a time.
            f.started.retain(|s| s.kind != "movie");
        }
        f.started.push(Started { kind, id, name: name.to_owned(), tick });
    });
}

fn add_wait(lua: &Lua, script: Table, kind: Kind) {
    flow(lua, |f| f.waits.push(Wait { script, kind }));
}

fn alloc_sound(lua: &Lua, name: &str) -> mlua::Result<u32> {
    flow(lua, |f| {
        let id = f.next_sound;
        f.next_sound = id
            .checked_add(1)
            .filter(|n| i32::try_from(*n).is_ok())
            .ok_or_else(|| mlua::Error::runtime(format!("A2.{name}: sound ids exhausted")))?;
        Ok(id)
    })
}

fn fade(lua: &Lua, ctx: &Shared, name: &'static str, out: bool) -> mlua::Result<mlua::Function> {
    let c = ctx.clone();
    lua.create_function(move |lua, args: MultiValue| {
        arity(name, &args, 5)?;
        let script = script_arg(name, &args[0])?;
        // `FadeIn/FadeOut(script, duration, delay, white, dontWait)`.
        let duration = seconds(name, "duration", &args[1])?;
        let delay = seconds(name, "delay", &args[2])?;
        let white = finite(name, "colour selector", &args[3])? != 0.0;
        let dont_wait = flag(name, &args[4])?;
        let rgb = if white { [255; 3] } else { [0; 3] };
        let fade = Presentation::Fade { out, seconds: Fixed::from_f64(duration), rgb };
        if delay == 0.0 {
            present(&c, name, fade)?;
        } else {
            // The typed fade has no start delay: the request is kept with every argument, and the
            // fade is presented when it starts (LUA-17).
            let arguments = vec![
                Arg::Num(Fixed::from_f64(duration)),
                Arg::Num(Fixed::from_f64(delay)),
                Arg::Bool(white),
            ];
            submit(&c, name, arguments)?;
            c.borrow_mut().present_later(delay, fade);
        }
        if !dont_wait {
            let due = c.borrow().view.tick + ((duration + delay) * f64::from(TICK_HZ)).ceil() as u64;
            add_wait(lua, script, Kind::Fade(due));
        }
        Ok(())
    })
}

fn sound_call(
    lua: &Lua,
    ctx: &Shared,
    name: &'static str,
    music: bool,
) -> mlua::Result<mlua::Function> {
    let c = ctx.clone();
    lua.create_function(move |lua, args: MultiValue| {
        arity(name, &args, 3)?;
        let script = script_arg(name, &args[0])?;
        let Arg::Text(file) = text(name, &args[1])? else { unreachable!("text() returns Text") };
        let dont_wait = flag(name, &args[2])?;
        // A waiting message is not started once the mission is over.
        if !dont_wait && c.borrow().finished {
            return Ok(-1_i64);
        }
        let id = alloc_sound(lua, name)?;
        let file_name = file.clone();
        let event = if music {
            Presentation::Call {
                name: format!("A2.{name}"),
                args: vec![Arg::Int(i64::from(id)), Arg::Text(file)],
            }
        } else {
            Presentation::Sound { id, name: file, at: None }
        };
        present(&c, name, event)?;
        c.borrow_mut().playback.sounds.insert(id);
        log_start(lua, &c, if music { "music" } else { "sound" }, i64::from(id), &file_name);
        if !dont_wait {
            add_wait(lua, script, Kind::Sound(id));
        }
        Ok(i64::from(id))
    })
}

pub(super) fn install(lua: &Lua, ctx: &Shared, table: &Table) -> mlua::Result<()> {
    table.set("FadeIn", fade(lua, ctx, "FadeIn", false)?)?;
    table.set("FadeOut", fade(lua, ctx, "FadeOut", true)?)?;
    table.set("AudioMessage", sound_call(lua, ctx, "AudioMessage", false)?)?;
    table.set("PlayMusic", sound_call(lua, ctx, "PlayMusic", true)?)?;

    let c = ctx.clone();
    table.set(
        "StopAudioMessage",
        lua.create_function(move |_, args: MultiValue| {
            arity("StopAudioMessage", &args, 1)?;
            let id = int32("StopAudioMessage", "sound id", &args[0])?;
            // The engine ignores an id that is no longer playing.
            let live = u32::try_from(id).is_ok_and(|id| c.borrow_mut().playback.sounds.remove(&id));
            if live {
                present(&c, "StopAudioMessage", Presentation::StopSound { id: id as u32 })?;
            }
            Ok(())
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "CineractiveStart",
        lua.create_function(move |lua, args: MultiValue| {
            arity("CineractiveStart", &args, 2)?;
            let script = script_arg("CineractiveStart", &args[0])?;
            let dont_wait = flag("CineractiveStart", &args[1])?;
            // The same singleton `Cineractive.active = true` drives.
            let singleton: mlua::AnyUserData = lua.globals().raw_get("Cineractive")?;
            // A start while one runs creates nothing and holds no reference (static).
            let running = super::cineractive_active(lua)?;
            singleton.set("active", true)?;
            if !dont_wait && !running {
                log_start(lua, &c, "cineractive", 0, "");
                add_wait(lua, script, Kind::Cineractive);
            }
            Ok(())
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "PlayCinematicMovie",
        lua.create_function(move |lua, args: MultiValue| {
            arity("PlayCinematicMovie", &args, 4)?;
            let script = script_arg("PlayCinematicMovie", &args[0])?;
            let Arg::Text(file) = text("PlayCinematicMovie", &args[1])? else {
                unreachable!("text() returns Text")
            };
            if file.is_empty() {
                return Err(mlua::Error::runtime("A2.PlayCinematicMovie requires a movie name"));
            }
            // Always 1 in the stock scripts and unused by the video start.
            int32("PlayCinematicMovie", "mode", &args[2])?;
            let dont_wait = flag("PlayCinematicMovie", &args[3])?;
            let movie_name = file.clone();
            present(
                &c,
                "PlayCinematicMovie",
                Presentation::Movie { name: file, in_panel: false, skippable: true },
            )?;
            c.borrow_mut().playback.movie = true;
            log_start(lua, &c, "movie", 0, &movie_name);
            if !dont_wait {
                add_wait(lua, script, Kind::Movie);
            }
            Ok(())
        })?,
    )?;

    let c = ctx.clone();
    table.set(
        "WaitForEnterKey",
        lua.create_function(move |lua, args: MultiValue| {
            arity("WaitForEnterKey", &args, 1)?;
            let script = script_arg("WaitForEnterKey", &args[0])?;
            let id = flow(lua, |f| {
                let id = f.next_key;
                f.next_key = id.saturating_add(1);
                id
            });
            present(&c, "WaitForEnterKey", Presentation::WaitForKey { id })?;
            log_start(lua, &c, "key", i64::from(id), "");
            add_wait(lua, script, Kind::Key(id));
            Ok(())
        })?,
    )?;

    // Step reference count of a script: waits not yet released.
    let c = ctx.clone();
    table.set(
        "ScriptWaits",
        lua.create_function(move |lua, args: MultiValue| {
            arity("ScriptWaits", &args, 1)?;
            let script = script_arg("ScriptWaits", &args[0])?;
            let ctx = c.borrow();
            Ok(flow(lua, |f| {
                f.waits.retain(|w| match w.kind {
                    Kind::Fade(due) => ctx.view.tick < due,
                    Kind::Sound(id) => ctx.playback.sounds.contains(&id),
                    Kind::Movie => ctx.playback.movie,
                    Kind::Key(_) | Kind::Cineractive => true,
                });
                f.waits.iter().filter(|w| w.script.to_pointer() == script.to_pointer()).count() as i64
            }))
        })?,
    )?;

    // What a host must still report (not a DLL call): a list of
    // `{kind, id, name, tick}` for sounds and music not yet finished, a movie
    // playing, Enter waits and cineractive starts held, oldest first.
    let c = ctx.clone();
    table.set(
        "HostPending",
        lua.create_function(move |lua, args: MultiValue| {
            arity("HostPending", &args, 0)?;
            let ctx = c.borrow();
            let out = lua.create_table()?;
            flow(lua, |f| {
                let waits = &f.waits;
                f.started.retain(|s| match s.kind {
                    "sound" | "music" => ctx.playback.sounds.contains(&(s.id as u32)),
                    "movie" => ctx.playback.movie,
                    "key" => waits.iter().any(|w| matches!(w.kind, Kind::Key(k) if i64::from(k) == s.id)),
                    "cineractive" => waits.iter().any(|w| matches!(w.kind, Kind::Cineractive)),
                    _ => false,
                });
                for (i, s) in f.started.iter().enumerate() {
                    let row = lua.create_table()?;
                    row.set("kind", s.kind)?;
                    row.set("id", s.id)?;
                    row.set("name", s.name.as_str())?;
                    row.set("tick", s.tick)?;
                    out.set(i + 1, row)?;
                }
                Ok::<(), mlua::Error>(())
            })?;
            Ok(out)
        })?,
    )?;

    // Host reports (not DLL calls).
    table.set(
        "EnterKeyDone",
        lua.create_function(move |lua, args: MultiValue| {
            arity("EnterKeyDone", &args, 1)?;
            let id = int32("EnterKeyDone", "id", &args[0])?;
            flow(lua, |f| f.waits.retain(|w| !matches!(w.kind, Kind::Key(k) if i64::from(k) == id)));
            Ok(())
        })?,
    )?;
    table.set(
        "CineractiveStartDone",
        lua.create_function(move |lua, args: MultiValue| {
            arity("CineractiveStartDone", &args, 0)?;
            flow(lua, |f| f.waits.retain(|w| !matches!(w.kind, Kind::Cineractive)));
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "native_flow/tests.rs"]
mod tests;
