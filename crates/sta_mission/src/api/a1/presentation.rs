//! Armada I presentation calls: bridge scenes and movies, cineractives,
//! camera shots, fades, titles, audio messages, the minimap and the
//! objectives display. Each queues `Edit::Present` from `PlayerId::SCRIPT`;
//! nothing here plays media or moves a camera.
//!
//! # Bridge scenes
//!
//! `BridgeDisplay::BridgeType` is an integer 0..=4: 1 Federation, 2 Klingon,
//! 3 Romulan, 4 Borg (measured from the stock DLLs); 0 is passed through with
//! an empty race name (not measured).
//!
//! The scene calls are `Presentation::Call` `A1.PreLoadBridgeScene` /
//! `A1.SetBridgeSceneRace` with `[Int type, Text race]` and
//! `A1.ShowBridgeScene` / `A1.ShowBridgeOnScreen` with `[Bool]`, for the
//! bridge display (MED-03 and UI) to consume. Movies use the typed events:
//! `PlayBridgeMovie(name, flag)` is `Presentation::Movie { in_panel: true,
//! skippable: true }`, `StopMovie` is `Presentation::StopMovie`. The flag is
//! validated and dropped (meaning not measured).
//!
//! # Completion
//!
//! `IsMoviePlaying` and `IsAudioMessageDone` need the client's answer. Until
//! a host reports it with `A1.MovieDone()` / `A1.AudioMessageDone(id)` a movie
//! or message counts as still playing until stopped.
//!
//! # Titles
//!
//! `ShowTitle` / `ShowSubtitle(x, y, width, height, text, flag)`: the
//! rectangle is in 640x480 screen pixels (measured), the flag is the 1 s
//! fade in (read from the executable). `RemoveTitle(id, true)` queues
//! `A1.RemoveTitleFade [Int id]` (1 s fade out), `false` the typed `RemoveTitle`. The typed `Presentation::Title` (no placement) is queued and the
//! rectangle is kept in `A1.ShowTitle` / `A1.ShowSubtitle` calls
//! `[Int id, x, y, width, height, Bool flag]`.

use mlua::{Function, Lua, ObjectLike, Table, Value};
use sta_sim::present::{Arg, Presentation};

use super::{A, Env, args_of, def, objects, rename_error};

const RACES: [&str; 5] = ["", "federation", "klingon", "romulan", "borg"];

fn bridge_type(name: &str, a: &A) -> mlua::Result<(i64, String)> {
    let n = a.int();
    let race = usize::try_from(n)
        .ok()
        .and_then(|i| RACES.get(i))
        .ok_or_else(|| mlua::Error::runtime(format!("A1.{name} requires a bridge type 0..=4")))?;
    Ok((n, (*race).to_owned()))
}

fn bridge_race(lua: &Lua, env: &Env, name: &'static str) -> mlua::Result<Function> {
    def(lua, env, name, &["i"], move |_, e, _, a| {
        let (n, race) = bridge_type(name, &a[0])?;
        let mut args = vec![Arg::Int(n)];
        if !race.is_empty() {
            args.push(Arg::Text(race));
        }
        e.call(name, args)?;
        Ok(Value::Nil)
    })
}

fn flag_call(lua: &Lua, env: &Env, name: &'static str) -> mlua::Result<Function> {
    def(lua, env, name, &["b"], move |_, e, _, a| {
        e.call(name, args_of(a))?;
        Ok(Value::Nil)
    })
}

fn cineractive(lua: &Lua, on: bool) -> mlua::Result<()> {
    // The same singleton the converted `Cineractive.active = on` drives.
    let singleton: mlua::AnyUserData = lua.globals().raw_get("Cineractive")?;
    singleton.set("active", on)
}

fn fade(lua: &Lua, env: &Env, name: &'static str, out: bool) -> mlua::Result<Function> {
    // `FadeIn/FadeOut(seconds, delay, f)`: the fade time and the delay before it starts, in
    // seconds, as A2's (`FadeOut(2, 25, 0)` at 22 s is black at 49 s in Federation 1: read from
    // the executable, static; LUA-17); the third is retained, meaning not measured. A delayed
    // fade is presented when it starts.
    def(lua, env, name, &["fff"], move |_, e, _, a| {
        e.call(name, args_of(a))?;
        let (seconds, delay) = (a[0].num(), a[1].num());
        if seconds >= 0.0 {
            let fade = Presentation::Fade { out, seconds: sta_core::Fixed::from_f64(seconds), rgb: [0, 0, 0] };
            if delay > 0.0 {
                e.ctx.borrow_mut().present_later(delay, fade);
            } else {
                e.present(name, fade)?;
            }
        }
        Ok(Value::Nil)
    })
}

fn title(lua: &Lua, env: &Env, name: &'static str, subtitle: bool) -> mlua::Result<Function> {
    def(lua, env, name, &["iiiisb"], move |_, e, _, a| {
        let id = {
            let mut st = e.st.borrow_mut();
            st.next_title = st
                .next_title
                .checked_add(1)
                .ok_or_else(|| mlua::Error::runtime(format!("A1.{name}: title ids exhausted")))?;
            st.next_title
        };
        e.present(name, Presentation::Title { id, text: a[4].text().to_owned(), subtitle, seconds: None })?;
        e.call(
            name,
            vec![Arg::Int(i64::from(id)), Arg::Int(a[0].int()), Arg::Int(a[1].int()), Arg::Int(a[2].int()), Arg::Int(a[3].int()), Arg::Bool(a[5].boolean())],
        )?;
        Ok(Value::Integer(i64::from(id)))
    })
}

fn remove_title(lua: &Lua, env: &Env, name: &'static str) -> mlua::Result<Function> {
    // `RemoveTitle(id, bool)`: true fades the title out over a second, then removes it; false
    // removes it at once (read from the executable, static).
    def(lua, env, name, &["ib"], move |_, e, _, a| {
        // 0 is the zeroed handle of a title the mission never showed (stock missions remove
        // their title handles on every path, shown or not: read from data, LUA-17): nothing to
        // remove.
        if a[0].int() == 0 {
            return Ok(Value::Nil);
        }
        let id = u32::try_from(a[0].int()).ok().filter(|id| *id >= 1 && *id <= e.st.borrow().next_title);
        let Some(id) = id else {
            return Err(mlua::Error::runtime(format!("A1.{name}: {} is not a title id this mission was given", a[0].int())));
        };
        if a[1].boolean() {
            e.call("RemoveTitleFade", vec![Arg::Int(i64::from(id))])?;
        } else {
            e.present(name, Presentation::RemoveTitle { id })?;
        }
        Ok(Value::Nil)
    })
}

fn plain_call(lua: &Lua, env: &Env, name: &'static str, specs: &'static [&'static str]) -> mlua::Result<Function> {
    def(lua, env, name, specs, move |_, e, _, a| {
        e.call(name, args_of(a))?;
        Ok(Value::Nil)
    })
}

pub(super) fn install(lua: &Lua, env: &Env, a2: &Table, table: &Table) -> mlua::Result<()> {
    // Bridge scenes and movies.
    table.set("PreLoadBridgeScene", bridge_race(lua, env, "PreLoadBridgeScene")?)?;
    table.set("SetBridgeSceneRace", bridge_race(lua, env, "SetBridgeSceneRace")?)?;
    table.set("ShowBridgeScene", flag_call(lua, env, "ShowBridgeScene")?)?;
    table.set("ShowBridgeOnScreen", flag_call(lua, env, "ShowBridgeOnScreen")?)?;
    table.set("PlayBridgeMovie", def(lua, env, "PlayBridgeMovie", &["si"], |_, e, _, a| {
        let name = a[0].text();
        if name.is_empty() {
            return Err(mlua::Error::runtime("A1.PlayBridgeMovie requires a movie name"));
        }
        e.present("PlayBridgeMovie", Presentation::Movie { name: name.to_owned(), in_panel: true, skippable: true })?;
        let tick = e.ctx.borrow().view.tick;
        let mut st = e.st.borrow_mut();
        st.movie_playing = true;
        st.started.retain(|s| s.0 != "movie");
        st.started.push(("movie", 0, name.to_owned(), tick));
        Ok(Value::Nil)
    })?)?;
    table.set("StopMovie", def(lua, env, "StopMovie", &[""], |_, e, _, _| {
        e.present("StopMovie", Presentation::StopMovie)?;
        let mut st = e.st.borrow_mut();
        st.movie_playing = false;
        st.started.retain(|s| s.0 != "movie");
        Ok(Value::Nil)
    })?)?;
    table.set("IsMoviePlaying", def(lua, env, "IsMoviePlaying", &[""], |_, e, _, _| {
        Ok(Value::Boolean(e.st.borrow().movie_playing))
    })?)?;
    // Host-side completion reports (not DLL calls). `HostPending()` lists what a
    // host must still report, `{kind, id, name, tick}` oldest first: sounds not
    // yet done or stopped and a movie still playing (as `A2.HostPending`).
    table.set("HostPending", def(lua, env, "HostPending", &[""], |lua, e, _, _| {
        let mut st = e.st.borrow_mut();
        let (sounds, movie) = (st.sounds_active.clone(), st.movie_playing);
        st.started.retain(|s| match s.0 {
            "sound" => u32::try_from(s.1).is_ok_and(|id| sounds.contains(&id)),
            _ => movie,
        });
        let out = lua.create_table()?;
        for (i, (kind, id, name, tick)) in st.started.iter().enumerate() {
            let row = lua.create_table()?;
            row.set("kind", *kind)?;
            row.set("id", *id)?;
            row.set("name", name.as_str())?;
            row.set("tick", *tick)?;
            out.set(i + 1, row)?;
        }
        Ok(Value::Table(out))
    })?)?;
    table.set("MovieDone", def(lua, env, "MovieDone", &[""], |_, e, _, _| {
        let mut st = e.st.borrow_mut();
        st.movie_playing = false;
        st.started.retain(|s| s.0 != "movie");
        Ok(Value::Nil)
    })?)?;
    table.set("AudioMessageDone", def(lua, env, "AudioMessageDone", &["i"], |_, e, _, a| {
        let mut st = e.st.borrow_mut();
        st.sounds_active.remove(&(a[0].int() as u32));
        let active = st.sounds_active.clone();
        st.started.retain(|s| s.0 != "sound" || u32::try_from(s.1).is_ok_and(|id| active.contains(&id)));
        Ok(Value::Nil)
    })?)?;

    // Cineractives and camera.
    table.set("CineractiveStart", def(lua, env, "CineractiveStart", &[""], |lua, _, _, _| {
        cineractive(lua, true)?;
        Ok(Value::Nil)
    })?)?;
    table.set("CineractiveEnd", def(lua, env, "CineractiveEnd", &[""], |lua, e, _, _| {
        // The fade belongs to the cinematic view (`ScreenFade` is in that view's render list
        // only: read from the executable, static): one still waiting out its delay has no view
        // to darken once the cineractive is over, so it is dropped (Federation 1's
        // `FadeOut(2, 25, 0)` scheduled at 22 s must not black out the game at 47 s after a
        // skip or an early end).
        e.ctx.borrow_mut().delayed.clear();
        cineractive(lua, false)?;
        Ok(Value::Nil)
    })?)?;
    table.set("FadeIn", fade(lua, env, "FadeIn", false)?)?;
    table.set("FadeOut", fade(lua, env, "FadeOut", true)?)?;
    table.set("CineractiveCameraOffset", def(lua, env, "CineractiveCameraOffset", &["sfb", "fffb"], |_, e, _, a| {
        e.call("CineractiveCameraOffset", args_of(a))?;
        Ok(Value::Nil)
    })?)?;
    // Seven overloads: `(from, at)` handles; `(path, point-or-time, handle)`;
    // `(path, point-or-time, path, point-or-time)`. The numbers keep their Lua
    // subtype and no meaning is assigned to them (as for A2).
    table.set("CineractiveShot", def(lua, env, "CineractiveShot", &["hh", "snh", "snsn"], |_, e, _, a| {
        for value in a {
            if let A::Handle(id) = value {
                e.unit("CineractiveShot", *id)?;
            }
        }
        e.call("CineractiveShot", args_of(a))?;
        Ok(Value::Nil)
    })?)?;
    table.set("CenterCamera", def(lua, env, "CenterCamera", &["si"], |_, e, _, a| {
        e.call("CenterCamera", args_of(a))?;
        Ok(Value::Nil)
    })?)?;

    // Titles, audio, minimap, objectives display.
    table.set("ShowTitle", title(lua, env, "ShowTitle", false)?)?;
    table.set("ShowSubtitle", title(lua, env, "ShowSubtitle", true)?)?;
    table.set("RemoveTitle", remove_title(lua, env, "RemoveTitle")?)?;
    table.set("RemoveSubtitle", remove_title(lua, env, "RemoveSubtitle")?)?;
    table.set("AudioMessage", def(lua, env, "AudioMessage", &["s"], |_, e, _, a| {
        let id = {
            let mut st = e.st.borrow_mut();
            st.next_sound = st
                .next_sound
                .checked_add(1)
                .ok_or_else(|| mlua::Error::runtime("A1.AudioMessage: sound ids exhausted"))?;
            let id = st.next_sound;
            st.sounds_active.insert(id);
            id
        };
        e.present("AudioMessage", Presentation::Sound { id, name: a[0].text().to_owned(), at: None })?;
        let tick = e.ctx.borrow().view.tick;
        e.st.borrow_mut().started.push(("sound", i64::from(id), a[0].text().to_owned(), tick));
        Ok(Value::Integer(i64::from(id)))
    })?)?;
    table.set("StopAudioMessage", def(lua, env, "StopAudioMessage", &["i"], |_, e, _, a| {
        // The engine looks the id up in its playing sounds and does nothing when
        // it is absent (read from Armada.exe, static): the missions stop handles
        // still holding their -1 start value.
        let Ok(id) = u32::try_from(a[0].int()) else { return Ok(Value::Nil) };
        if !e.st.borrow_mut().sounds_active.remove(&id) {
            return Ok(Value::Nil);
        }
        e.present("StopAudioMessage", Presentation::StopSound { id })?;
        Ok(Value::Nil)
    })?)?;
    table.set("IsAudioMessageDone", def(lua, env, "IsAudioMessageDone", &["i"], |_, e, _, a| {
        let active = u32::try_from(a[0].int()).is_ok_and(|id| e.st.borrow().sounds_active.contains(&id));
        Ok(Value::Boolean(!active))
    })?)?;
    table.set("MinimapMessage", plain_call(lua, env, "MinimapMessage", &["iif", "sifi"])?)?;
    table.set("GridVisible", plain_call(lua, env, "GridVisible", &["b"])?)?;
    table.set("RemoveBillboardEffect", plain_call(lua, env, "RemoveBillboardEffect", &["i"])?)?;
    table.set("FadeRemove", plain_call(lua, env, "FadeRemove", &[""])?)?;
    table.set("StartCredits", plain_call(lua, env, "StartCredits", &[""])?)?;
    table.set("EndCredits", plain_call(lua, env, "EndCredits", &[""])?)?;
    table.set("Starfield_Load_Background_Geometry", plain_call(lua, env, "Starfield_Load_Background_Geometry", &["s"])?)?;
    table.set("AddTransporterEffect", def(lua, env, "AddTransporterEffect", &["h"], |_, e, _, a| {
        objects::transporter(e, a[0].id())?;
        Ok(Value::Nil)
    })?)?;

    let visibility: Function = a2.get("ObjectivesDisplay_Set_Visibility")?;
    table.set("ObjectivesDisplay_Set_Visibility", def(lua, env, "ObjectivesDisplay_Set_Visibility", &["b"], move |_, _, _, a| {
        visibility.call::<()>(a[0].boolean()).map_err(rename_error)?;
        Ok(Value::Nil)
    })?)?;
    // The A1 call takes only the file; a fresh load resets goal states.
    let load: Function = a2.get("ObjectivesDisplay_Set_Text_From_File")?;
    table.set("ObjectivesDisplay_Set_Text_From_File", def(lua, env, "ObjectivesDisplay_Set_Text_From_File", &["s"], move |_, _, _, a| {
        load.call::<()>((a[0].text().to_owned(), true)).map_err(rename_error)?;
        Ok(Value::Nil)
    })?)?;
    Ok(())
}
